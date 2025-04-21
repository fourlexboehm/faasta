use anyhow::{Context, Result};
use lers::{solver::dns::CloudflareDns01Solver, Directory};
use std::fs;
use std::path::{PathBuf};
use std::time::{Duration, SystemTime};
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;
use tracing::{info, warn};
use openssl::pkey::PKey;

// Renew certificate if it expires in less than 30 days
const CERT_RENEWAL_DAYS: u64 = 30;

// Define constant URLs for Let's Encrypt
const LETS_ENCRYPT_URL: &str = "https://acme-v02.api.letsencrypt.org/directory";
const LETS_ENCRYPT_STAGING_URL: &str = "https://acme-staging-v02.api.letsencrypt.org/directory";

pub struct CertManager {
    domain: String,
    cert_path: PathBuf,
    key_path: PathBuf,
    account_key_path: PathBuf,
    email: String,
    use_staging: bool,
}

impl CertManager {
    pub fn new(
        domain: String,
        certs_dir: PathBuf,
        cert_path: PathBuf,
        key_path: PathBuf,
        email: String,
        use_staging: bool,
    ) -> Self {
        // Make sure the certs directory exists
        if !certs_dir.exists() {
            fs::create_dir_all(&certs_dir).expect("Failed to create certificates directory");
        }

        let account_key_path = certs_dir.join("acme_account.key");

        Self {
            domain,
            cert_path,
            key_path,
            account_key_path,
            email,
            use_staging,
        }
    }

    // Check if certificate needs renewal based on expiry date
    fn needs_cert_renewal(&self) -> Result<bool> {
        // If cert doesn't exist, we need to renew
        if !self.cert_path.exists() {
            info!("Certificate file doesn't exist, will create it");
            return Ok(true);
        }

        // Check certificate expiration date
        match self.get_expiry_time() {
            Ok(expiry) => {
                let now = SystemTime::now();
                match expiry.duration_since(now) {
                    Ok(time_left) => {
                        let days_left = time_left.as_secs() / (24 * 60 * 60);
                        info!("Certificate expires in {} days", days_left);
                        // Renew if less than CERT_RENEWAL_DAYS left
                        Ok(days_left < CERT_RENEWAL_DAYS)
                    }
                    Err(_) => {
                        // If expiry is in the past, we need to renew
                        info!("Certificate has already expired");
                        Ok(true)
                    }
                }
            }
            Err(e) => {
                warn!("Error checking certificate expiry: {}", e);
                // If we can't read the certificate, assume it needs renewal
                Ok(true)
            }
        }
    }

    // Get certificate expiry time
    fn get_expiry_time(&self) -> Result<SystemTime> {
        let cert_data = fs::read(&self.cert_path)
            .with_context(|| format!("Failed to read certificate file: {:?}", self.cert_path))?;

        let mut reader = std::io::Cursor::new(cert_data);
        let certs = rustls_pemfile::certs(&mut reader)
            .collect::<Result<Vec<_>, _>>()
            .context("Failed to parse certificate")?;

        if certs.is_empty() {
            anyhow::bail!("No certificates found in file: {:?}", self.cert_path);
        }

        // Get the first certificate's expiry time
        let x509 = x509_parser::parse_x509_certificate(&certs[0])
            .map_err(|e| anyhow::anyhow!("Failed to parse X.509 certificate: {}", e))?
            .1;

        let validity = x509.validity();
        let not_after = validity.not_after.to_datetime(); 
        
        // Convert to SystemTime
        let unix_seconds = not_after.unix_timestamp();
        let system_time = SystemTime::UNIX_EPOCH + Duration::from_secs(unix_seconds as u64);
        
        Ok(system_time)
    }

    // Obtain or renew the certificate
    pub async fn obtain_or_renew_certificate(&self) -> Result<()> {
        info!("Checking if certificate needs renewal for domain: {}", self.domain);
        
        // Check if certificate is expiring soon
        let needs_renewal = self.needs_cert_renewal()?;
        
        if !needs_renewal {
            info!("Certificate is still valid for more than {} days, skipping renewal", CERT_RENEWAL_DAYS);
            return Ok(());
        }
        
        // Certificate is expiring soon or doesn't exist, proceed with renewal
        info!("Certificate is expiring in less than {} days or doesn't exist, will renew", CERT_RENEWAL_DAYS);
        
        // Configure Let's Encrypt client
        info!("Setting up ACME client");
        let solver = CloudflareDns01Solver::from_env()?.build()?;
        
        let dir_url = if self.use_staging {
            LETS_ENCRYPT_STAGING_URL
        } else {
            LETS_ENCRYPT_URL
        };
        
        let directory = Directory::builder(dir_url)
            .dns01_solver(Box::new(solver))
            .build()
            .await?;
        
        // Get or create ACME account
        let account = if self.account_key_path.exists() {
            // Load existing account key
            let account_key_data = fs::read(&self.account_key_path)
                .with_context(|| format!("Failed to read account key: {:?}", self.account_key_path))?;
            
            // Parse the PEM-encoded private key
            let key = PKey::private_key_from_pem(&account_key_data)
                .with_context(|| "Failed to parse account private key")?;
            
            // Access existing account
            info!("Using existing ACME account");
            directory.account()
                .private_key(key)
                .contacts(vec![format!("mailto:{}", self.email)])
                .terms_of_service_agreed(true)
                .create_if_not_exists()
                .await?
        } else {
            // Create new account with auto-generated key
            info!("Creating new ACME account");
            let account = directory.account()
                .contacts(vec![format!("mailto:{}", self.email)])
                .terms_of_service_agreed(true)
                .create_if_not_exists()
                .await?;
            
            // Save the account key
            let private_key = account.private_key();
            let pem = private_key.private_key_to_pem_pkcs8()
                .with_context(|| "Failed to convert private key to PEM")?;
            fs::write(&self.account_key_path, pem)?;
            
            account
        };
        
        // Create or use private key for certificate
        let certificate = if self.key_path.exists() {
            // Use existing private key for the certificate
            info!("Using existing private key for certificate");
            
            // Load the existing key
            let key_data = fs::read(&self.key_path)
                .with_context(|| format!("Failed to read private key: {:?}", self.key_path))?;
            
            // Parse it for use with lers
            let key = PKey::private_key_from_pem(&key_data)
                .with_context(|| "Failed to parse private key")?;
            
            // Obtain new certificate with the existing key
            account.certificate()
                .add_domain(format!("*.{}", self.domain))
                .add_domain(self.domain.clone())
                .private_key(key)
                .obtain()
                .await?
        } else {
            // First time creation - generate new certificate and key
            info!("No existing private key found, generating new one (first-time setup)");
            account.certificate()
                .add_domain(format!("*.{}", self.domain))
                .add_domain(self.domain.clone())
                .obtain()
                .await?
        };
        
        // Save the certificate file
        info!("Saving certificate file");
        let mut cert_file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&self.cert_path)
            .await
            .with_context(|| format!("Failed to open cert file for writing: {:?}", self.cert_path))?;
        
        cert_file.write_all(&certificate.to_pem()?).await?;
        
        // Save private key only if it doesn't exist (first-time setup)
        if !self.key_path.exists() {
            info!("Creating private key file (first-time setup)");
            let mut key_file = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600) // Ensure proper permissions
                .open(&self.key_path)
                .await
                .with_context(|| format!("Failed to open key file for writing: {:?}", self.key_path))?;
            
            key_file.write_all(&certificate.private_key_to_pem()?).await?;
        } else {
            info!("Keeping existing private key (never regenerated)");
        }
        
        info!("Successfully renewed certificate for: {}", self.domain);
        Ok(())
    }
}
