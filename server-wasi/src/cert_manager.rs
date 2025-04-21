use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;
use tracing::{info, warn};

// Porkbun API response structure
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PorkbunResponse {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(rename = "certificatechain")]
    pub certificate_chain: Option<String>,
    #[serde(rename = "privatekey")]
    pub private_key: Option<String>,
    #[serde(rename = "publickey")]
    pub public_key: Option<String>,
    #[serde(rename = "intermediatecertificate")]
    pub intermediate_certificate: Option<String>,
}

// API request
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PorkbunRequest {
    pub apikey: String,
    pub secretapikey: String,
}

pub struct CertManager {
    domain: String,
    cert_path: PathBuf,
    key_path: PathBuf,
    client: Client,
}

impl CertManager {
    pub fn new(
        domain: String,
        certs_dir: PathBuf,
        cert_path: PathBuf,
        key_path: PathBuf,
    ) -> Self {
        // Make sure the certs directory exists
        if !certs_dir.exists() {
            fs::create_dir_all(&certs_dir).expect("Failed to create certificates directory");
        }

        Self {
            domain,
            cert_path,
            key_path, 
            client: Client::new(),
        }
    }

    // Check if certificate needs renewal based on expiry date
    fn needs_cert_renewal(&self) -> Result<bool> {
        // If cert doesn't exist, we need to renew
        if !self.cert_path.exists() {
            info!("Certificate file doesn't exist, will download it");
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
                        // Renew if less than 30 days left
                        Ok(days_left < 30)
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

        let mut reader = std::io::Cursor::new(&cert_data);
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

    // Retrieve SSL certificate from Porkbun API
    async fn get_ssl(&self) -> Result<PorkbunResponse> {
        // Get API keys from environment variables
        let apikey = match env::var("PORKBUN_API_KEY") {
            Ok(key) => key,
            Err(_) => return Err(anyhow::anyhow!("PORKBUN_API_KEY environment variable not set. Please set it to your Porkbun API key.")),
        };
        
        let secretapikey = match env::var("PORKBUN_SECRET_API_KEY") {
            Ok(key) => key,
            Err(_) => return Err(anyhow::anyhow!("PORKBUN_SECRET_API_KEY environment variable not set. Please set it to your Porkbun Secret API key.")),
        };
            
        let url = format!("https://api.porkbun.com/api/json/v3/ssl/retrieve/{}", self.domain);
        
        // Log some debug info (mask the actual API keys for security)
        let apikey_masked = format!("{}...{}", 
            apikey.chars().take(4).collect::<String>(), 
            &apikey[apikey.len().saturating_sub(4)..]);
            
        let secretkey_masked = format!("{}...{}", 
            secretapikey.chars().take(4).collect::<String>(), 
            &secretapikey[secretapikey.len().saturating_sub(4)..]);
            
        info!("Using Porkbun API keys: {} and {}", apikey_masked, secretkey_masked);
        
        let request_body = PorkbunRequest {
            apikey: apikey.clone(),
            secretapikey: secretapikey.clone(),
        };
        
        info!("Sending request to Porkbun API for domain: {}", self.domain);
        
        let response = self.client
            .post(&url)
            .json(&request_body)
            .send()
            .await
            .context("Failed to send request to Porkbun API")?;
            
        // Get the raw response text first for debugging
        let response_text = response
            .text()
            .await
            .context("Failed to read response body from Porkbun API")?;
        
        info!("Received response from Porkbun API: {}", &response_text);
            
        // Parse the response
        let response_json: PorkbunResponse = serde_json::from_str(&response_text)
            .context(format!("Failed to parse Porkbun API response: {}", response_text))?;

        if response_json.status == "ERROR" {
            return Err(anyhow::anyhow!(
                "Error retrieving SSL from Porkbun: {}",
                response_json.message.unwrap_or_else(|| "Unknown error".to_string())
            ));
        }

        Ok(response_json)
    }

    // Obtain or renew the certificate
    pub async fn obtain_or_renew_certificate(&self) -> Result<()> {
        info!(
            "Checking if certificate needs renewal for domain: {}",
            self.domain
        );

        // Check if certificate is expiring soon
        let needs_renewal = self.needs_cert_renewal()?;

        if !needs_renewal {
            info!(
                "Certificate is still valid for more than 30 days, skipping renewal"
            );
            return Ok(());
        }

        // Get certificates from Porkbun API
        info!("Downloading certificates for domain: {}", self.domain);
        let cert_json = self.get_ssl().await?;

        // Save domain certificate
        if let Some(cert_chain) = cert_json.certificate_chain {
            info!("Installing domain certificate to {:?}", self.cert_path);
            let mut cert_file = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&self.cert_path)
                .await?;
            cert_file.write_all(cert_chain.as_bytes()).await?;
        } else {
            return Err(anyhow::anyhow!("Certificate chain missing in Porkbun API response"));
        }

        // Save private key
        if let Some(private_key) = cert_json.private_key {
            info!("Installing private key to {:?}", self.key_path);
            let mut key_file = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600) // Ensure proper permissions
                .open(&self.key_path)
                .await?;
            key_file.write_all(private_key.as_bytes()).await?;
        } else {
            return Err(anyhow::anyhow!("Private key missing in Porkbun API response"));
        }

        info!("Successfully downloaded certificates for domain: {}", self.domain);
        Ok(())
    }
}
