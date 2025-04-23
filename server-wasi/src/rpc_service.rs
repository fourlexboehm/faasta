use crate::github_auth::GitHubAuth;
use crate::metrics::get_metrics;
use crate::SERVER;
use dashmap::DashMap;
use faasta_interface::{FunctionError, FunctionInfo, FunctionResult, FunctionService, Metrics};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{debug, error};

/// Sled tree name for function metadata
const FUNCTIONS_DB_TREE: &str = "functions";

/// Implementation of the FunctionService
/// The FaastaServer struct is the one holding the pre_cache, but we need a way to
/// clear cache entries when unpublishing functions.
///
#[derive(Clone)]
pub struct FunctionServiceImpl {
    functions_dir: PathBuf,
    functions_db: Arc<DashMap<String, FunctionInfo>>,
    github_auth: Arc<GitHubAuth>,
    functions_tree: sled::Tree,
}

impl FunctionServiceImpl {
    /// Create a new FunctionServiceImpl
    /// Create a new FunctionServiceImpl, loading persisted metadata from sled
    pub fn new(
        functions_dir: PathBuf,
        github_auth: Arc<GitHubAuth>,
        metadata_db: sled::Db,
    ) -> anyhow::Result<Self> {
        // Ensure functions directory exists
        if !functions_dir.exists() {
            fs::create_dir_all(&functions_dir)?;
        }

        // In-memory function info
        let functions_db = Arc::new(DashMap::new());

        // Open or create sled tree for function metadata
        let functions_tree = metadata_db.open_tree(FUNCTIONS_DB_TREE)?;

        // Load existing function metadata from sled
        for item in functions_tree.iter() {
            let (key_bytes, val) = item?;
            let name = String::from_utf8(key_bytes.to_vec())?;
            // Decode using bincode
            match bincode::decode_from_slice::<FunctionInfo, _>(&val, bincode::config::standard()) {
                Ok((info, _)) => {
                    functions_db.insert(name, info);
                }
                Err(e) => {
                    error!("Failed to decode function metadata for {}: {}", name, e);
                }
            }
        }

        Ok(Self {
            functions_dir,
            functions_db,
            github_auth,
            functions_tree,
        })
    }

    /// Validate GitHub authentication token
    async fn validate_auth(&self, username: &str, token: &str) -> anyhow::Result<bool> {
        // Check if the token is in the format "username:token"
        if let Some((_, token_value)) = token.split_once(':') {
            // If we have the username:token format, extract just the token part
            return self
                .github_auth
                .validate_oauth_token(username, token_value)
                .await;
        }

        // If the token is not in the expected format, use it as is
        // This maintains backward compatibility with other token formats
        self.github_auth.validate_oauth_token(username, token).await
    }

    /// Extract username from GitHub token
    async fn get_username_from_token(&self, token: &str) -> FunctionResult<String> {
        // Check if the token is in the format "username:token"
        if let Some((username, token_value)) = token.split_once(':') {
            // If we already have the username in the token format, validate it with GitHub
            // Extract token from "Bearer {token}" format if present
            let token_value = token_value.strip_prefix("Bearer ").unwrap_or(token_value);

            // Create client to verify with GitHub API
            let client = reqwest::Client::new();
            let response = client
                .get("https://api.github.com/user")
                .header("User-Agent", "faasta-server")
                .header("Authorization", format!("Bearer {}", token_value))
                .send()
                .await
                .map_err(|e| {
                    FunctionError::AuthError(format!("Failed to contact GitHub API: {}", e))
                })?;

            if !response.status().is_success() {
                return Err(FunctionError::AuthError(format!(
                    "GitHub API returned error status: {}",
                    response.status()
                )));
            }

            // Verify the username matches what GitHub returns
            let github_user: serde_json::Value = response.json().await.map_err(|e| {
                FunctionError::AuthError(format!("Failed to parse GitHub response: {}", e))
            })?;

            let api_username = github_user["login"].as_str().ok_or_else(|| {
                FunctionError::AuthError("Username not found in GitHub response".to_string())
            })?;

            if username != api_username {
                return Err(FunctionError::AuthError(
                    "Username mismatch in GitHub authentication".to_string(),
                ));
            }

            return Ok(username.to_string());
        }

        // Fallback to the old method if the token is not in the expected format
        // Extract token from "Bearer {token}" format if present
        let token_value = token.strip_prefix("Bearer ").unwrap_or(token);

        // Create client to verify with GitHub API
        let client = reqwest::Client::new();
        let response = client
            .get("https://api.github.com/user")
            .header("User-Agent", "faasta-server")
            .header("Authorization", format!("Bearer {}", token_value))
            .send()
            .await
            .map_err(|e| {
                FunctionError::AuthError(format!("Failed to contact GitHub API: {}", e))
            })?;

        if !response.status().is_success() {
            return Err(FunctionError::AuthError(format!(
                "GitHub API returned error status: {}",
                response.status()
            )));
        }

        // Extract username from response
        let github_user: serde_json::Value = response.json().await.map_err(|e| {
            FunctionError::AuthError(format!("Failed to parse GitHub response: {}", e))
        })?;

        let username = github_user["login"].as_str().ok_or_else(|| {
            FunctionError::AuthError("Username not found in GitHub response".to_string())
        })?;

        Ok(username.to_string())
    }
}

impl FunctionService for FunctionServiceImpl {
    async fn publish(
        self,
        _: tarpc::context::Context,
        wasm_file: Vec<u8>,
        name: String,
        github_auth_token: String,
    ) -> FunctionResult<String> {
        // Extract username from token
        let username = self.get_username_from_token(&github_auth_token).await?;

        // Validate token
        if !self
            .validate_auth(&username, &github_auth_token)
            .await
            .map_err(|e| FunctionError::AuthError(e.to_string()))?
        {
            return Err(FunctionError::AuthError(
                "Invalid GitHub authentication token".to_string(),
            ));
        }

        // Check if function name is valid
        if name.is_empty()
            || !name
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
        {
            return Err(FunctionError::InvalidInput(
                "Invalid function name. Use only alphanumeric characters, underscores, and hyphens.".to_string()
            ));
        }

        // Check WASM file size
        if wasm_file.len() > faasta_interface::MAX_WASM_SIZE {
            return Err(FunctionError::InvalidInput(format!(
                "WASM file too large. Maximum allowed size is 30MB, but received {} bytes",
                wasm_file.len()
            )));
        }

        // Simple direct approach: use the exact function name for the WASM file
        let wasm_filename = format!("{}.wasm", name);
        let wasm_path = self.functions_dir.join(&wasm_filename);

        // Check if function already exists
        if self.functions_db.contains_key(&name) || wasm_path.exists() {
            if let Some(entry) = self.functions_db.get(&name) {
                // Check if user owns the function
                if entry.owner != username {
                    return Err(FunctionError::PermissionDenied(
                        "A function with this name already exists and belongs to another user"
                            .to_string(),
                    ));
                }
                // Function exists and user owns it - proceed with update
            } else {
                // Function exists on disk but not in memory db - this is inconsistent state
                // Still enforce ownership check through GitHub auth
                return Err(FunctionError::PermissionDenied(
                    "A function with this name already exists. Please choose a different name."
                        .to_string(),
                ));
            }
        } else {
            // New function - enforce project limit
            if !self.github_auth.can_upload_project(&username, &name) {
                return Err(FunctionError::PermissionDenied(
                    "You have reached the maximum limit of 10 projects".to_string(),
                ));
            }
            // Register ownership
            match self.github_auth.add_project(&username, &name).await {
                Ok(_) => debug!("Added project '{}' for user '{}'", name, username),
                Err(e) => {
                    error!("Failed to add project: {}", e);
                    return Err(FunctionError::InternalError(format!(
                        "Failed to add project: {}",
                        e
                    )));
                }
            }
        }

        // When publishing a new version, clear any existing cache entry
        if let Some(server) = SERVER.get() {
            server.remove_from_cache(&name);
        }

        // Write the WASM file
        let mut file = fs::File::create(&wasm_path)
            .map_err(|e| FunctionError::InternalError(format!("Failed to create file: {}", e)))?;
        file.write_all(&wasm_file)
            .map_err(|e| FunctionError::InternalError(format!("Failed to write file: {}", e)))?;

        // Create function info
        let now = chrono::Utc::now().to_rfc3339();
        let function_info = FunctionInfo {
            name: name.clone(),
            owner: username,
            published_at: now,
            usage: format!("https://{}.faasta.xyz", name),
        };

        // Save in-memory and persist metadata to sled
        self.functions_db
            .insert(name.clone(), function_info.clone());
        // Serialize metadata with bincode
        let meta =
            bincode::encode_to_vec(&function_info, bincode::config::standard()).map_err(|e| {
                FunctionError::InternalError(format!(
                    "Failed to serialize function metadata: {}",
                    e
                ))
            })?;
        // Persist metadata to sled
        self.functions_tree
            .insert(name.as_bytes(), meta)
            .map_err(|e| {
                FunctionError::InternalError(format!("Failed to persist function metadata: {}", e))
            })?;

        Ok(format!("Function '{}' published successfully", name))
    }

    async fn list_functions(
        self,
        _: tarpc::context::Context,
        github_auth_token: String,
    ) -> FunctionResult<Vec<FunctionInfo>> {
        // Extract username from token
        let username = self.get_username_from_token(&github_auth_token).await?;

        // Validate token
        if !self
            .validate_auth(&username, &github_auth_token)
            .await
            .map_err(|e| FunctionError::AuthError(e.to_string()))?
        {
            return Err(FunctionError::AuthError(
                "Invalid GitHub authentication token".to_string(),
            ));
        }

        // Filter functions by owner
        let user_functions: Vec<FunctionInfo> = self
            .functions_db
            .iter()
            .filter(|entry| entry.owner == username)
            .map(|entry| entry.clone())
            .collect();

        Ok(user_functions)
    }

    async fn unpublish(
        self,
        _: tarpc::context::Context,
        name: String,
        github_auth_token: String,
    ) -> FunctionResult<()> {
        // Extract username from token
        let username = self.get_username_from_token(&github_auth_token).await?;

        // Validate token
        if !self
            .validate_auth(&username, &github_auth_token)
            .await
            .map_err(|e| FunctionError::AuthError(e.to_string()))?
        {
            return Err(FunctionError::AuthError(
                "Invalid GitHub authentication token".to_string(),
            ));
        }

        // Check if function exists
        if let Some(entry) = self.functions_db.get(&name) {
            // Check if user owns the function
            if entry.owner != username {
                return Err(FunctionError::PermissionDenied(
                    "You don't have permission to unpublish this function".to_string(),
                ));
            }

            // Remove function from database
            self.functions_db.remove(&name);

            // Remove WASM file using direct name
            let wasm_filename = format!("{}.wasm", name);
            let wasm_path = self.functions_dir.join(wasm_filename);
            if wasm_path.exists() {
                fs::remove_file(wasm_path).map_err(|e| {
                    FunctionError::InternalError(format!("Failed to remove file: {}", e))
                })?;
            }

            // Remove the project from the user's list
            match self.github_auth.remove_project(&username, &name).await {
                Ok(_) => {
                    debug!("Removed project '{}' for user '{}'", name, username);
                }
                Err(e) => {
                    error!("Failed to remove project: {}", e);
                    // We don't return an error here because the function was already removed
                    // Just log the error
                }
            }

            // Remove metadata from sled
            match self.functions_tree.remove(name.as_bytes()) {
                Ok(_) => debug!("Successfully removed metadata for function '{}'", name),
                Err(e) => error!("Failed to remove function metadata for '{}': {}", name, e),
                // We don't return an error here because the function was already removed
            }

            Ok(())
        } else {
            Err(FunctionError::NotFound(format!(
                "Function '{}' not found",
                name
            )))
        }
    }

    async fn get_metrics(
        self,
        _: tarpc::context::Context,
        github_auth_token: String,
    ) -> FunctionResult<Metrics> {
        // Extract username from token
        let username = self.get_username_from_token(&github_auth_token).await?;

        // Validate token
        if !self
            .validate_auth(&username, &github_auth_token)
            .await
            .map_err(|e| FunctionError::AuthError(e.to_string()))?
        {
            return Err(FunctionError::AuthError(
                "Invalid GitHub authentication token".to_string(),
            ));
        }

        // Use the metrics module to get metrics from sled
        let metrics = get_metrics();

        Ok(metrics)
    }
}

/// Helper function to create a service implementation with GitHub auth
pub fn create_service_with_github_auth(
    functions_dir: PathBuf,
    github_auth: Arc<GitHubAuth>,
    metadata_db: sled::Db,
) -> anyhow::Result<FunctionServiceImpl> {
    FunctionServiceImpl::new(functions_dir, github_auth, metadata_db)
}
