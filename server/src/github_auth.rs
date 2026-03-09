use anyhow::Result;
use bincode::{Decode, Encode};
use cyper::Client as HttpClient;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::storage;

const MAX_PROJECTS_PER_USER: usize = 10;
const USER_AGENT: &str = "faasta-server";

pub struct GitHubAuth;

#[derive(Serialize, Deserialize, Clone, Debug, Encode, Decode)]
pub struct UserData {
    pub github_username: String,
    pub projects: Vec<String>,
}

impl GitHubAuth {
    pub async fn new() -> Result<Self> {
        Ok(Self)
    }

    pub async fn authenticate_github(&self, token: &str) -> Result<(String, bool)> {
        let (provided_username, token_value) =
            if let Some((username, token_part)) = token.split_once(':') {
                (
                    Some(username.to_string()),
                    token_part
                        .strip_prefix("Bearer ")
                        .unwrap_or(token_part)
                        .trim(),
                )
            } else {
                (None, token.strip_prefix("Bearer ").unwrap_or(token).trim())
            };

        let request = match HttpClient::new().get("https://api.github.com/user") {
            Ok(builder) => builder,
            Err(err) => {
                tracing::error!("Failed to create GitHub request builder: {}", err);
                return Ok(("".to_string(), false));
            }
        };

        let request = match request.header("User-Agent", USER_AGENT) {
            Ok(builder) => builder,
            Err(err) => {
                tracing::error!("Failed to set GitHub User-Agent header: {}", err);
                return Ok(("".to_string(), false));
            }
        };

        let request = match request.header("Authorization", format!("Bearer {token_value}")) {
            Ok(builder) => builder,
            Err(err) => {
                tracing::error!("Failed to set GitHub Authorization header: {}", err);
                return Ok(("".to_string(), false));
            }
        };

        let response = match request.send().await {
            Ok(resp) => resp,
            Err(err) => {
                tracing::error!("GitHub API request failed: {}", err);
                return Ok(("".to_string(), false));
            }
        };

        if !response.status().is_success() {
            tracing::warn!("GitHub API returned error status: {}", response.status());
            return Ok(("".to_string(), false));
        }

        let github_user: Value = match response.json().await {
            Ok(json) => json,
            Err(err) => {
                tracing::error!("Failed to parse GitHub response: {}", err);
                return Ok(("".to_string(), false));
            }
        };

        let api_username = github_user["login"].as_str().unwrap_or("");

        if let Some(provided) = provided_username
            && provided != api_username
        {
            tracing::warn!(
                "Username mismatch: provided '{}', GitHub returned '{}'",
                provided,
                api_username
            );
            return Ok((api_username.to_string(), false));
        }

        Ok((api_username.to_string(), true))
    }

    pub async fn can_upload_project(&self, username: &str, project_name: &str) -> Result<bool> {
        let Some(user_data) = self.get_user_data(username)? else {
            return Ok(true);
        };

        Ok(user_data.projects.len() < MAX_PROJECTS_PER_USER
            || user_data.projects.contains(&project_name.to_string()))
    }

    pub async fn add_project(&self, username: &str, project_name: &str) -> Result<()> {
        let mut user_data = self.get_user_data(username)?.unwrap_or(UserData {
            github_username: username.to_string(),
            projects: Vec::new(),
        });

        if !user_data.projects.contains(&project_name.to_string()) {
            user_data.projects.push(project_name.to_string());
        }

        let encoded = bincode::encode_to_vec(&user_data, bincode::config::standard())?;
        storage::put_user(username, &encoded)
    }

    pub async fn remove_project(&self, username: &str, project_name: &str) -> Result<()> {
        if let Some(mut user_data) = self.get_user_data(username)? {
            user_data.projects.retain(|p| p != project_name);
            let encoded = bincode::encode_to_vec(&user_data, bincode::config::standard())?;
            storage::put_user(username, &encoded)?;
        }
        Ok(())
    }

    pub async fn get_user_projects(&self, username: &str) -> Result<Option<Vec<String>>> {
        Ok(self
            .get_user_data(username)?
            .map(|user_data| user_data.projects))
    }

    fn get_user_data(&self, username: &str) -> Result<Option<UserData>> {
        let Some(encoded) = storage::get_user(username)? else {
            return Ok(None);
        };

        let (user_data, _) =
            bincode::decode_from_slice::<UserData, _>(&encoded, bincode::config::standard())?;
        Ok(Some(user_data))
    }
}
