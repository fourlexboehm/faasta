use crate::github_auth::GitHubAuth;
use crate::metrics::get_metrics;
use crate::SERVER;
use faasta_interface::{FunctionError, FunctionInfo, FunctionResult, FunctionService, Metrics};
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{debug, error, info};

/// Sled tree name for function metadata
const FUNCTIONS_DB_TREE: &str = "functions";

/// Implementation of the FunctionService
/// The FaastaServer struct is the one holding the pre_cache, but we need a way to
/// clear cache entries when unpublishing functions.
///
#[derive(Clone)]
pub struct FunctionServiceImpl {
    functions_dir: PathBuf,
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

        // Open or create sled tree for function metadata
        let functions_tree = metadata_db.open_tree(FUNCTIONS_DB_TREE)?;

        Ok(Self {
            functions_dir,
            github_auth,
            functions_tree,
        })
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
        // Use the new combined authentication function
        let (username, is_valid) = self
            .github_auth
            .authenticate_github(&github_auth_token)
            .await
            .map_err(|e| FunctionError::AuthError(format!("Authentication error: {}", e)))?;

        if !is_valid || username.is_empty() {
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
        if wasm_path.exists() {
            let entry_result = self.functions_tree.get(name.as_bytes()).map_err(|e| {
                FunctionError::InternalError(format!("Failed to get function metadata: {}", e))
            })?;

            if let Some(entry_bytes) = entry_result {
                // Deserialize the function info
                let function_info = match bincode::decode_from_slice::<FunctionInfo, _>(
                    &entry_bytes,
                    bincode::config::standard(),
                ) {
                    Ok((info, _)) => info,
                    Err(e) => {
                        error!("Failed to deserialize function info: {}", e);
                        return Err(FunctionError::InternalError(format!(
                            "Failed to deserialize function info: {}",
                            e
                        )));
                    }
                };

                // Check if user owns the function
                if function_info.owner != username {
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

        // Create function info with both subdomain and path-based URLs
        let now = chrono::Utc::now().to_rfc3339();
        let function_info = FunctionInfo {
            name: name.clone(),
            owner: username,
            published_at: now,
            usage: format!("https://{}.faasta.xyz or https://faasta.xyz/{}", name, name),
        };

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
        // Use the new combined authentication function
        let (username, is_valid) = self
            .github_auth
            .authenticate_github(&github_auth_token)
            .await
            .map_err(|e| FunctionError::AuthError(format!("Authentication error: {}", e)))?;

        if !is_valid || username.is_empty() {
            return Err(FunctionError::AuthError(
                "Invalid GitHub authentication token".to_string(),
            ));
        }

        // Filter functions by owner
        // Get from sled and deserialize with bincode
        let user_functions: Vec<FunctionInfo> = self
            .functions_tree
            .iter()
            .filter_map(|res| {
                if let Ok((_, value)) = res {
                    // Deserialize the function info
                    match bincode::decode_from_slice::<FunctionInfo, _>(
                        &value,
                        bincode::config::standard(),
                    ) {
                        Ok((function_info, _)) => {
                            // Filter by owner
                            if function_info.owner == username {
                                Some(function_info)
                            } else {
                                None
                            }
                        }
                        Err(e) => {
                            error!("Failed to deserialize function info: {}", e);
                            None
                        }
                    }
                } else {
                    None
                }
            })
            .collect();

        Ok(user_functions)
    }

    async fn unpublish(
        self,
        _: tarpc::context::Context,
        name: String,
        github_auth_token: String,
    ) -> FunctionResult<()> {
        info!("Processing unpublish request for function: {}", name);

        // Use the new combined authentication function
        let (username, is_valid) = self
            .github_auth
            .authenticate_github(&github_auth_token)
            .await
            .map_err(|e| {
                error!("Authentication error during unpublish: {}", e);
                FunctionError::AuthError(format!("Authentication error: {}", e))
            })?;

        if !is_valid || username.is_empty() {
            error!("Invalid authentication token for unpublish operation");
            return Err(FunctionError::AuthError(
                "Invalid GitHub authentication token".to_string(),
            ));
        }

        info!("Authentication successful for user: {}", username);

        // Check if function exists
        let entry_result = self.functions_tree.get(name.as_bytes()).map_err(|e| {
            FunctionError::InternalError(format!("Failed to get function metadata: {}", e))
        })?;

        if let Some(entry_bytes) = entry_result {
            // Deserialize the function info
            let function_info = match bincode::decode_from_slice::<FunctionInfo, _>(
                &entry_bytes,
                bincode::config::standard(),
            ) {
                Ok((info, _)) => info,
                Err(e) => {
                    error!("Failed to deserialize function info: {}", e);
                    return Err(FunctionError::InternalError(format!(
                        "Failed to deserialize function info: {}",
                        e
                    )));
                }
            };

            // Check if user owns the function
            if function_info.owner != username {
                error!(
                    "Permission denied: function owned by {} but requested by {}",
                    function_info.owner, username
                );
                return Err(FunctionError::PermissionDenied(
                    "You don't have permission to unpublish this function".to_string(),
                ));
            }

            // Remove WASM file using direct name
            let wasm_filename = format!("{}.wasm", name);
            let wasm_path = self.functions_dir.join(wasm_filename);
            if wasm_path.exists() {
                if let Err(e) = fs::remove_file(&wasm_path) {
                    error!("Failed to remove WASM file: {}", e);
                } else {
                    debug!("Successfully removed WASM file for function '{}'", name);
                }
            }

            // Remove metadata from sled
            match self.functions_tree.remove(name.as_bytes()) {
                Ok(_) => debug!("Successfully removed metadata for function '{}'", name),
                Err(e) => error!("Failed to remove function metadata for '{}': {}", name, e),
                // We don't return an error here because the function was already removed
            }

            // Remove the project from the user's list
            match self.github_auth.remove_project(&username, &name).await {
                Ok(_) => {
                    debug!("Removed project '{}' for user '{}'", name, username);
                }
                Err(e) => {
                    error!("Failed to remove project: {}", e);
                }
            }

            info!("Function '{}' unpublished successfully", name);
            Ok(())
        } else {
            error!("Function '{}' not found for unpublish operation", name);
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
        // Use the new combined authentication function
        let (username, is_valid) = self
            .github_auth
            .authenticate_github(&github_auth_token)
            .await
            .map_err(|e| FunctionError::AuthError(format!("Authentication error: {}", e)))?;

        if !is_valid || username.is_empty() {
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
