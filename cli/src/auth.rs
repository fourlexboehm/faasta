use anyhow::Error;
use compio::buf::BufResult;
use cyper::Client as HttpClient;
use dirs::config_dir;
use http::header::HeaderMap;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const CONFIG_FILE: &str = "faasta/github_auth.json";
const USER_AGENT: &str = "faasta-cli";
const MACHINE_MAN_PREVIEW: &str = "application/vnd.github.machine-man-preview+json";
/// Installation access tokens expire after an hour; refresh a little early.
const TOKEN_REFRESH_INTERVAL: Duration = Duration::from_secs(55 * 60);

#[derive(Debug, Serialize)]
struct JwtClaims {
    iat: u64,
    exp: u64,
    iss: u64,
}

#[derive(Clone)]
struct GithubAuthParams {
    user_agent: String,
    private_key: Vec<u8>,
    app_id: u64,
    installation_id: u64,
}

#[derive(Deserialize)]
struct RawInstallationAccessToken {
    token: String,
}

/// GitHub app installation token, refreshed automatically before it expires.
struct InstallationAccessToken {
    token: String,
    refresh_at: Instant,
    params: GithubAuthParams,
}

impl InstallationAccessToken {
    async fn new(params: GithubAuthParams) -> Result<Self, Error> {
        let raw = fetch_installation_token(&params).await?;
        Ok(Self {
            token: raw.token,
            refresh_at: Instant::now() + TOKEN_REFRESH_INTERVAL,
            params,
        })
    }

    async fn header(&mut self) -> Result<HeaderMap, Error> {
        if Instant::now() >= self.refresh_at {
            let raw = fetch_installation_token(&self.params).await?;
            self.token = raw.token;
            self.refresh_at = Instant::now() + TOKEN_REFRESH_INTERVAL;
        }

        let mut headers = HeaderMap::new();
        headers.insert(
            http::header::AUTHORIZATION,
            format!("token {}", self.token).parse()?,
        );
        Ok(headers)
    }
}

async fn fetch_installation_token(
    params: &GithubAuthParams,
) -> Result<RawInstallationAccessToken, Error> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let claims = JwtClaims {
        iat: now,
        exp: now + 60,
        iss: params.app_id,
    };
    let header = Header {
        alg: Algorithm::RS256,
        ..Default::default()
    };
    let key = EncodingKey::from_rsa_pem(&params.private_key)?;
    let jwt = jsonwebtoken::encode(&header, &claims, &key)?;

    let url = format!(
        "https://api.github.com/app/installations/{}/access_tokens",
        params.installation_id
    );
    let response = HttpClient::new()?
        .post(&url)?
        .bearer_auth(jwt)?
        .header("Accept", MACHINE_MAN_PREVIEW)?
        .header("User-Agent", params.user_agent.as_str())?
        .send()
        .await?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("GitHub installation token request failed ({status}): {body}");
    }

    Ok(response.json().await?)
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct AuthConfig {
    pub app_id: u64,
    pub installation_id: u64,
    pub private_key: Vec<u8>,
    pub user_id: Option<String>,
    pub project_hmacs: HashMap<String, String>, // project_name -> hmac
}

/// Manages GitHub authentication for the CLI
pub struct GitHubAuth {
    config: AuthConfig,
    token: Option<InstallationAccessToken>,
    config_path: PathBuf,
}

impl GitHubAuth {
    /// Initialize GitHub authentication
    pub async fn new() -> Result<Self, Error> {
        let config_path = Self::get_config_path()?;
        let config = Self::load_config(&config_path).await?;

        Ok(Self {
            config,
            token: None,
            config_path,
        })
    }

    /// Authenticate with GitHub
    pub async fn authenticate(&mut self) -> Result<(), Error> {
        if self.config.app_id == 0
            || self.config.installation_id == 0
            || self.config.private_key.is_empty()
        {
            return Err(anyhow::anyhow!(
                "GitHub App not configured. Run 'cargo faasta auth setup' first."
            ));
        }

        self.token = Some(
            InstallationAccessToken::new(GithubAuthParams {
                user_agent: USER_AGENT.into(),
                private_key: self.config.private_key.clone(),
                app_id: self.config.app_id,
                installation_id: self.config.installation_id,
            })
            .await?,
        );

        // Get user ID if we don't have it yet
        if self.config.user_id.is_none() {
            self.fetch_and_store_user_id().await?;
        }

        Ok(())
    }

    /// Get authentication header for API requests
    pub async fn header(&mut self) -> Result<HeaderMap, Error> {
        if self.token.is_none() {
            self.authenticate().await?;
        }

        self.token.as_mut().unwrap().header().await
    }

    /// Store project HMAC for ownership verification
    pub async fn store_project_hmac(
        &mut self,
        project_name: &str,
        hmac: &str,
    ) -> Result<(), Error> {
        self.config
            .project_hmacs
            .insert(project_name.to_string(), hmac.to_string());
        self.save_config().await?;
        Ok(())
    }

    /// Get project HMAC if it exists
    pub fn get_project_hmac(&self, project_name: &str) -> Option<&String> {
        self.config.project_hmacs.get(project_name)
    }

    /// Check if a project is owned by the current user
    pub fn owns_project(&self, project_name: &str) -> bool {
        self.config.project_hmacs.contains_key(project_name)
    }

    /// Get the list of projects owned by the user
    pub fn get_owned_projects(&self) -> Vec<String> {
        self.config.project_hmacs.keys().cloned().collect()
    }

    /// Check if user has reached project limit
    pub fn has_reached_project_limit(&self) -> bool {
        self.config.project_hmacs.len() >= 5
    }

    /// Setup GitHub app credentials
    pub async fn setup(
        &mut self,
        app_id: u64,
        installation_id: u64,
        private_key: Vec<u8>,
    ) -> Result<(), Error> {
        self.config.app_id = app_id;
        self.config.installation_id = installation_id;
        self.config.private_key = private_key;
        self.save_config().await?;
        Ok(())
    }

    /// Get user ID from authenticated GitHub instance
    async fn fetch_and_store_user_id(&mut self) -> Result<(), Error> {
        let header = self.token.as_mut().unwrap().header().await?;

        // Create authenticated client
        let response = HttpClient::new()?
            .get("https://api.github.com/app")?
            .headers(header)
            .send()
            .await?;

        if response.status().is_success() {
            let app_info: serde_json::Value = response.json().await?;
            if let Some(id) = app_info.get("id").and_then(|v| v.as_str()) {
                self.config.user_id = Some(id.to_string());
                self.save_config().await?;
            }
        }

        Ok(())
    }

    /// Get the path to the config file
    fn get_config_path() -> Result<PathBuf, Error> {
        let mut path =
            config_dir().ok_or_else(|| anyhow::anyhow!("Could not find user config directory"))?;

        path.push(CONFIG_FILE);

        // Ensure parent directory exists
        if let Some(parent) = path.parent()
            && !parent.exists()
        {
            fs::create_dir_all(parent)?;
        }

        Ok(path)
    }

    /// Load config from disk
    async fn load_config(path: &Path) -> Result<AuthConfig, Error> {
        if path.exists() {
            let data = compio::fs::read(path).await?;
            let content = String::from_utf8(data)?;
            Ok(serde_json::from_str(&content)?)
        } else {
            // Create default config
            let default_config = AuthConfig::default();
            let content = serde_json::to_string_pretty(&default_config)?;
            let BufResult(result, _) = compio::fs::write(path, content.into_bytes()).await;
            result?;
            Ok(default_config)
        }
    }

    /// Save config to disk
    pub async fn save_config(&self) -> Result<(), Error> {
        let content = serde_json::to_string_pretty(&self.config)?;
        let BufResult(result, _) = compio::fs::write(&self.config_path, content.into_bytes()).await;
        result?;
        Ok(())
    }

    /// Check if GitHub app is configured
    pub fn is_configured(&self) -> bool {
        self.config.app_id != 0
            && self.config.installation_id != 0
            && !self.config.private_key.is_empty()
    }

    /// Get user ID if available
    pub fn get_user_id(&self) -> Option<&str> {
        self.config.user_id.as_deref()
    }
}
