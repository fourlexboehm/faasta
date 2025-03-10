use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use dashmap::DashMap;
use github_app_auth::{GithubAuthParams, InstallationAccessToken};
use tokio::sync::Mutex;
use tokio::fs;
use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const USER_DATA_DIR: &str = "./user_data";
const MAX_PROJECTS_PER_USER: usize = 10;

/// Struct to hold GitHub auth configuration
pub struct GitHubAuth {
    user_projects: DashMap<String, UserData>,
    tokens: Mutex<HashMap<String, Arc<Mutex<InstallationAccessToken>>>>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct UserData {
    pub github_username: String,
    pub projects: Vec<String>,
}

impl GitHubAuth {
    pub async fn new() -> Result<Self> {
        // Create the user data directory if it doesn't exist
        if !Path::new(USER_DATA_DIR).exists() {
            fs::create_dir_all(USER_DATA_DIR).await?;
        }
        
        // Load existing user data
        let user_projects = DashMap::new();
        let user_data_dir = PathBuf::from(USER_DATA_DIR);
        
        if user_data_dir.exists() {
            let mut entries = fs::read_dir(user_data_dir).await?;
            
            while let Some(entry) = entries.next_entry().await? {
                if entry.file_type().await?.is_file() && entry.path().extension().map_or(false, |ext| ext == "json") {
                    if let Ok(file_content) = fs::read_to_string(entry.path()).await {
                        if let Ok(user_data) = serde_json::from_str::<UserData>(&file_content) {
                            user_projects.insert(user_data.github_username.clone(), user_data);
                        }
                    }
                }
            }
        }
        
        Ok(Self {
            user_projects,
            tokens: Mutex::new(HashMap::new()),
        })
    }
    
    /// Validate OAuth token directly with GitHub API
    pub async fn validate_oauth_token(&self, username: &str, token: &str) -> Result<bool> {
        // Extract token from "Bearer {token}" format if present
        let token_value = token.strip_prefix("Bearer ").unwrap_or(token);
        
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
            if user_data.projects.len() >= MAX_PROJECTS_PER_USER && !user_data.projects.contains(&project_name.to_string()) {
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
        self.user_projects.insert(username.to_string(), user_data.clone());
        
        // Save to disk
        let file_path = PathBuf::from(USER_DATA_DIR).join(format!("{}.json", username));
        fs::write(file_path, serde_json::to_string_pretty(&user_data)?).await?;
        
        Ok(())
    }
    
    /// Verify that a user owns a function using the stored HMAC
    pub fn verify_function_ownership(&self, username: &str, function_name: &str) -> bool {
        if let Some(user_data) = self.user_projects.get(username) {
            return user_data.projects.contains(&function_name.to_string());
        }
        false
    }
    
    /// Get user data by function name
    pub fn get_user_by_function(&self, function_name: &str) -> Option<String> {
        for entry in self.user_projects.iter() {
            if entry.projects.contains(&function_name.to_string()) {
                return Some(entry.github_username.clone());
            }
        }
        None
    }
}
