use crate::function_runtime::{self, SERVER};
use crate::metrics::get_metrics;
use crate::storage;
use faasta_interface::{FunctionError, FunctionInfo, FunctionResult, FunctionService, Metrics};
use std::fs;
use std::hash::{Hash, Hasher};
use tracing::{debug, error, info};

/// Implementation of the FunctionService
/// The FaastaServer struct is the one holding the pre_cache, but we need a way to
/// clear cache entries when unpublishing functions.
///
#[derive(Clone)]
pub struct FunctionServiceImpl;

impl FunctionServiceImpl {
    /// Create a new FunctionServiceImpl
    pub fn new() -> anyhow::Result<Self> {
        // Ensure functions directory exists
        let server = SERVER.get().unwrap();
        if !server.functions_dir.exists() {
            fs::create_dir_all(&server.functions_dir)?;
        }

        Ok(Self)
    }
}

// Helper implementation that uses references to avoid cloning
impl FunctionServiceImpl {
    fn validate_artifact_bytes(name: &str, artifact_bytes: &[u8]) -> FunctionResult<()> {
        let server = SERVER.get().unwrap();
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        name.hash(&mut hasher);
        artifact_bytes.hash(&mut hasher);
        let nonce = hasher.finish();
        let validation_dir = server.functions_dir.join(".validation");
        let validation_path = validation_dir.join(format!("{name}-{nonce:016x}.so"));

        fs::create_dir_all(&validation_dir).map_err(|e| {
            FunctionError::InternalError(format!(
                "Failed to create validation directory {}: {e}",
                validation_dir.display()
            ))
        })?;

        fs::write(&validation_path, artifact_bytes).map_err(|e| {
            FunctionError::InternalError(format!(
                "Failed to write validation artifact {}: {e}",
                validation_path.display()
            ))
        })?;

        let symbol_name = function_runtime::function_symbol_name(name);
        let validation_result = unsafe {
            let library = libloading::Library::new(&validation_path).map_err(|e| {
                FunctionError::InvalidInput(format!(
                    "Uploaded artifact is not a valid shared library: {e}"
                ))
            })?;
            let validation_result: Result<(), FunctionError> = library
                .get::<libloading::Symbol<*const ()>>(symbol_name.as_bytes())
                .map(|_| ())
                .map_err(|e| {
                    FunctionError::InvalidInput(format!(
                        "Shared library is missing expected symbol '{symbol_name}': {e}"
                    ))
                });
            drop(library);
            validation_result
        };

        if let Err(err) = fs::remove_file(&validation_path)
            && err.kind() != std::io::ErrorKind::NotFound
        {
            debug!(
                "failed to remove validation artifact {}: {}",
                validation_path.display(),
                err
            );
        }

        validation_result
    }

    pub(crate) async fn publish_impl(
        &self,
        artifact_bytes: Vec<u8>,
        name: String,
        github_auth_token: String,
    ) -> FunctionResult<String> {
        // Use the new combined authentication function
        let server = SERVER.get().unwrap();
        let (username, is_valid) = server
            .github_auth
            .authenticate_github(&github_auth_token)
            .await
            .map_err(|e| FunctionError::AuthError(format!("Authentication error: {e}")))?;

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
        if artifact_bytes.len() > faasta_interface::MAX_WASM_SIZE {
            return Err(FunctionError::InvalidInput(format!(
                "Artifact too large. Maximum allowed size is 30MB, but received {} bytes",
                artifact_bytes.len()
            )));
        }

        let existing_entry = storage::get_function(&name).map_err(|e| {
            FunctionError::InternalError(format!("Failed to get function metadata: {e}"))
        })?;

        let is_new_function = existing_entry.is_none();

        if let Some(entry_bytes) = existing_entry {
            let function_info = match bincode::decode_from_slice::<FunctionInfo, _>(
                &entry_bytes,
                bincode::config::standard(),
            ) {
                Ok((info, _)) => info,
                Err(e) => {
                    error!("Failed to deserialize function info: {}", e);
                    return Err(FunctionError::InternalError(format!(
                        "Failed to deserialize function info: {e}"
                    )));
                }
            };

            if function_info.owner != username {
                return Err(FunctionError::PermissionDenied(
                    "A function with this name already exists and belongs to another user"
                        .to_string(),
                ));
            }
        } else {
            if !server
                .github_auth
                .can_upload_project(&username, &name)
                .await
                .map_err(|e| {
                    FunctionError::InternalError(format!(
                        "Failed to check project upload permissions: {e}"
                    ))
                })?
            {
                return Err(FunctionError::PermissionDenied(
                    "You have reached the maximum limit of 10 projects".to_string(),
                ));
            }
        }

        Self::validate_artifact_bytes(&name, &artifact_bytes)?;

        storage::put_artifact(&name, &artifact_bytes).map_err(|e| {
            FunctionError::InternalError(format!("Failed to persist artifact bytes: {e}"))
        })?;

        // Create function info with both subdomain and path-based URLs
        let now = chrono::Utc::now().to_rfc3339();
        let function_info = FunctionInfo {
            name: name.clone(),
            owner: username.clone(),
            published_at: now,
            usage: format!("https://{name}.faasta.lol or https://faasta.lol/{name}"),
        };

        let meta =
            bincode::encode_to_vec(&function_info, bincode::config::standard()).map_err(|e| {
                FunctionError::InternalError(format!("Failed to serialize function metadata: {e}"))
            })?;
        if let Err(err) = storage::put_function(&name, &meta) {
            let _ = storage::delete_artifact(&name);
            return Err(FunctionError::InternalError(format!(
                "Failed to persist function metadata: {err}"
            )));
        }

        if is_new_function && let Err(err) = server.github_auth.add_project(&username, &name).await
        {
            error!("Failed to add project: {}", err);
            let _ = storage::delete_function(&name);
            let _ = storage::delete_artifact(&name);
            return Err(FunctionError::InternalError(format!(
                "Failed to add project: {err}"
            )));
        }

        server.invalidate_function(&name).await;

        Ok(format!("Function '{name}' published successfully"))
    }

    pub(crate) async fn list_functions_impl(
        &self,
        github_auth_token: String,
    ) -> FunctionResult<Vec<FunctionInfo>> {
        // Use the new combined authentication function
        let server = SERVER.get().unwrap();
        let (username, is_valid) = server
            .github_auth
            .authenticate_github(&github_auth_token)
            .await
            .map_err(|e| FunctionError::AuthError(format!("Authentication error: {e}")))?;

        if !is_valid || username.is_empty() {
            return Err(FunctionError::AuthError(
                "Invalid GitHub authentication token".to_string(),
            ));
        }

        // Get the user's projects from the user_tree
        let mut user_functions = Vec::new();

        // Get user data to find which projects they own
        if let Some(projects) = server
            .github_auth
            .get_user_projects(&username)
            .await
            .map_err(|e| {
                FunctionError::InternalError(format!("Failed to get user projects: {e}"))
            })?
        {
            // For each project owned by the user, get the function info
            for project_name in projects {
                // Get function info from the functions tree
                if let Ok(Some(value)) = storage::get_function(&project_name) {
                    // Deserialize the function info
                    match bincode::decode_from_slice::<FunctionInfo, _>(
                        &value,
                        bincode::config::standard(),
                    ) {
                        Ok((function_info, _)) => {
                            user_functions.push(function_info);
                        }
                        Err(e) => {
                            error!("Failed to deserialize function info for '{project_name}': {e}");
                        }
                    }
                }
            }
        }

        Ok(user_functions)
    }

    pub(crate) async fn unpublish_impl(
        &self,
        name: String,
        github_auth_token: String,
    ) -> FunctionResult<()> {
        info!("Processing unpublish request for function: {name}");

        let server = SERVER.get().unwrap();
        // Use the new combined authentication function
        let (username, is_valid) = server
            .github_auth
            .authenticate_github(&github_auth_token)
            .await
            .map_err(|e| {
                error!("Authentication error during unpublish: {e}");
                FunctionError::AuthError(format!("Authentication error: {e}"))
            })?;

        if !is_valid || username.is_empty() {
            error!("Invalid authentication token for unpublish operation");
            return Err(FunctionError::AuthError(
                "Invalid GitHub authentication token".to_string(),
            ));
        }

        info!("Authentication successful for user: {username}");

        // Check if function exists
        let entry_result = storage::get_function(&name).map_err(|e| {
            FunctionError::InternalError(format!("Failed to get function metadata: {e}"))
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
                        "Failed to deserialize function info: {e}"
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

            storage::delete_artifact(&name).map_err(|e| {
                error!("Failed to remove artifact bytes for '{name}': {e}");
                FunctionError::InternalError(format!("Failed to remove artifact bytes: {e}"))
            })?;
            debug!("Successfully removed artifact bytes for function '{name}'");

            storage::delete_function(&name).map_err(|e| {
                error!("Failed to remove function metadata for '{name}': {e}");
                FunctionError::InternalError(format!("Failed to remove function metadata: {e}"))
            })?;
            debug!("Successfully removed metadata for function '{name}'");

            server
                .github_auth
                .remove_project(&username, &name)
                .await
                .map_err(|e| {
                    error!("Failed to remove project: {e}");
                    FunctionError::InternalError(format!("Failed to remove project ownership: {e}"))
                })?;
            debug!("Removed project '{name}' for user '{username}'");

            server.invalidate_function(&name).await;

            info!("Function '{name}' unpublished successfully");
            Ok(())
        } else {
            error!("Function '{name}' not found for unpublish operation");
            Err(FunctionError::NotFound(format!(
                "Function '{name}' not found"
            )))
        }
    }

    pub(crate) async fn get_metrics_impl(
        &self,
        github_auth_token: String,
    ) -> FunctionResult<Metrics> {
        // Use the new combined authentication function
        let server = SERVER.get().unwrap();
        let (username, is_valid) = server
            .github_auth
            .authenticate_github(&github_auth_token)
            .await
            .map_err(|e| FunctionError::AuthError(format!("Authentication error: {e}")))?;

        if !is_valid || username.is_empty() {
            return Err(FunctionError::AuthError(
                "Invalid GitHub authentication token".to_string(),
            ));
        }

        // Use the metrics module to get persisted metrics
        let metrics = get_metrics();

        Ok(metrics)
    }
}

// Now implement the trait methods that use the reference-based implementations
#[bitrpc::async_trait]
impl FunctionService for FunctionServiceImpl {
    async fn publish(
        &self,
        artifact_bytes: Vec<u8>,
        name: String,
        github_auth_token: String,
    ) -> bitrpc::Result<FunctionResult<String>> {
        Ok(self
            .publish_impl(artifact_bytes, name, github_auth_token)
            .await)
    }

    async fn list_functions(
        &self,
        github_auth_token: String,
    ) -> bitrpc::Result<FunctionResult<Vec<FunctionInfo>>> {
        Ok(self.list_functions_impl(github_auth_token).await)
    }

    async fn unpublish(
        &self,
        name: String,
        github_auth_token: String,
    ) -> bitrpc::Result<FunctionResult<()>> {
        Ok(self.unpublish_impl(name, github_auth_token).await)
    }

    async fn get_metrics(
        &self,
        github_auth_token: String,
    ) -> bitrpc::Result<FunctionResult<Metrics>> {
        Ok(self.get_metrics_impl(github_auth_token).await)
    }
}

/// Helper function to create a service implementation with GitHub auth
pub fn create_service() -> anyhow::Result<FunctionServiceImpl> {
    use crate::metrics::Timer;
    use tracing::info;

    info!("Initializing RPC service...");
    let rpc_init_timer = Timer::new("rpc_service_initialization".to_string());
    let service = FunctionServiceImpl::new()?;
    drop(rpc_init_timer); // Explicitly drop to record timing
    info!("RPC service initialization complete");

    Ok(service)
}
