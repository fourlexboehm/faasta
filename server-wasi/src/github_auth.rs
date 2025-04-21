use anyhow::Result;
use bincode::{Decode, Encode};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;

const USER_DB_TREE: &str = "user_data";
const MAX_PROJECTS_PER_USER: usize = 10;

/// Struct to hold GitHub auth configuration
pub struct GitHubAuth {
    user_projects: DashMap<String, UserData>,
    db: sled::Db,
}
#[derive(Serialize, Deserialize, Clone, Debug, Encode, Decode)]
pub struct UserData {
    pub github_username: String,
    pub projects: Vec<String>,
}

impl GitHubAuth {
    pub async fn new(db: sled::Db) -> Result<Self> {
        // Load existing user data
        let user_projects = DashMap::new();

        // Create or get the user data tree
        let user_tree = db.open_tree(USER_DB_TREE)?;

        // Iterate through all items in the tree
        for item in user_tree.iter().flatten() {
            if let Ok(username) = std::str::from_utf8(&item.0) {
                // Try to decode using bincode
                if let Ok((user_data, _)) =
                    bincode::decode_from_slice::<UserData, _>(&item.1, bincode::config::standard())
                {
                    user_projects.insert(username.to_string(), user_data);
                } else {
                    // Fallback to serde_json for backward compatibility
                    if let Ok(user_data) = serde_json::from_slice::<UserData>(&item.1) {
                        user_projects.insert(username.to_string(), user_data);
                    }
                }
            }
        }

        Ok(Self { user_projects, db })
    }

    /// Validate OAuth token directly with GitHub API
    pub async fn validate_oauth_token(&self, username: &str, token: &str) -> Result<bool> {
        // Extract token from "Bearer {token}" format if present
        let token_value = token.strip_prefix("Bearer ").unwrap_or(token).trim();

        // Create client to verify with GitHub API
        let client = reqwest::Client::new();
        let response = client
            .get("https://api.github.com/user")
            .header("User-Agent", "faasta-server")
            .header("Authorization", format!("Bearer {}", token_value))
            .send()
            .await?;

        if !response.status().is_success() {
            return Ok(false);
        }

        // Verify username matches
        let github_user: Value = response.json().await?;
        let api_username = github_user["login"].as_str().unwrap_or("");

        Ok(api_username == username)
    }

    /// Check if a user can upload more projects (limit is MAX_PROJECTS_PER_USER)
    pub fn can_upload_project(&self, username: &str, project_name: &str) -> bool {
        if let Some(user_data) = self.user_projects.get(username) {
            // Check if they're already at the limit
            if user_data.projects.len() >= MAX_PROJECTS_PER_USER
                && !user_data.projects.contains(&project_name.to_string())
            {
                return false;
            }
        }
        true
    }

    /// Add a project to a user's list
    pub async fn add_project(&self, username: &str, project_name: &str) -> Result<()> {
        // Get or create user data
        let mut user_data = if let Some(data) = self.user_projects.get(username) {
            data.clone()
        } else {
            UserData {
                github_username: username.to_string(),
                projects: Vec::new(),
            }
        };

        // Add or update the project
        if !user_data.projects.contains(&project_name.to_string()) {
            user_data.projects.push(project_name.to_string());
        }

        // Update the map
        self.user_projects
            .insert(username.to_string(), user_data.clone());

        // Save to database
        let user_tree = self.db.open_tree(USER_DB_TREE)?;
        let encoded = bincode::encode_to_vec(&user_data, bincode::config::standard())?;
        user_tree.insert(username.as_bytes(), encoded)?;

        Ok(())
    }

    /// Remove a project from a user's list
    pub async fn remove_project(&self, username: &str, project_name: &str) -> Result<()> {
        // Get user data
        if let Some(mut user_data) = self.user_projects.get_mut(username) {
            // Remove the project
            user_data.projects.retain(|p| p != project_name);

            // Save to database
            let user_tree = self.db.open_tree(USER_DB_TREE)?;
            let user_data_clone = user_data.clone();
            let encoded = bincode::encode_to_vec(&user_data_clone, bincode::config::standard())?;
            user_tree.insert(username.as_bytes(), encoded)?;
        }

        Ok(())
    }
}
