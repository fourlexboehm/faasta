use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::Mutex;
use std::fs;
use std::io::Write;
use dashmap::DashMap;
use std::time::{Duration, UNIX_EPOCH};

// Define a custom error type that can be serialized
#[derive(Debug, Error, Serialize, Deserialize, Clone)]
pub enum FunctionError {
    #[error("Authentication error: {0}")]
    AuthError(String),
    
    #[error("Function not found: {0}")]
    NotFound(String),
    
    #[error("Permission denied: {0}")]
    PermissionDenied(String),
    
    #[error("Invalid input: {0}")]
    InvalidInput(String),
    
    #[error("Internal error: {0}")]
    InternalError(String),
}

// Type alias for Result with our custom error
pub type FunctionResult<T> = std::result::Result<T, FunctionError>;

// Define the data structures for our service

/// Represents a published function
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FunctionInfo {
    /// Name of the function
    pub name: String,
    /// Owner's GitHub username
    pub owner: String,
    /// When the function was published
    pub published_at: String,
    /// Usage information
    pub usage: String,
}

/// Function metrics information
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FunctionMetricsResponse {
    /// Name of the function
    pub function_name: String,
    /// Total execution time in milliseconds
    pub total_time_millis: u64,
    /// Number of times the function was called
    pub call_count: u64,
    /// Last time the function was called (ISO 8601 format)
    pub last_called: String,
}

/// Overall metrics information
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Metrics {
    /// Total execution time across all functions in milliseconds
    pub total_time: u64,
    /// Total number of function calls
    pub total_calls: u64,
    /// Metrics for individual functions
    pub function_metrics: Vec<FunctionMetricsResponse>,
}

/// Service interface for managing functions
#[tarpc::service]
pub trait FunctionService {
    /// Publish a new function
    async fn publish(wasm_file: Vec<u8>, name: String, github_auth_token: String) -> FunctionResult<String>;
    
    /// List all functions for the authenticated user
    async fn list_functions(github_auth_token: String) -> FunctionResult<Vec<FunctionInfo>>;
    
    /// Unpublish a function
    async fn unpublish(name: String, github_auth_token: String) -> FunctionResult<()>;
    
    /// Get metrics for all functions
    async fn get_metrics(github_auth_token: String) -> FunctionResult<Metrics>;
}

/// Implementation of the FunctionService
#[derive(Clone)]
pub struct FunctionServiceImpl {
    functions_dir: PathBuf,
    functions_db: Arc<DashMap<String, FunctionInfo>>,
    metrics_db: Arc<DashMap<String, (u64, u64, u64)>>, // (total_time, call_count, last_called)
    auth_validator: Arc<Mutex<Box<dyn Fn(&str, &str) -> anyhow::Result<bool> + Send + Sync>>>,
}

impl FunctionServiceImpl {
    /// Create a new FunctionServiceImpl
    pub fn new<F>(
        functions_dir: PathBuf,
        auth_validator: F,
    ) -> anyhow::Result<Self>
    where
        F: Fn(&str, &str) -> anyhow::Result<bool> + Send + Sync + 'static,
    {
        // Create functions directory if it doesn't exist
        if !functions_dir.exists() {
            fs::create_dir_all(&functions_dir)?;
        }
        
        // Load existing functions from the directory
        let functions_db = Arc::new(DashMap::new());
        
        // TODO: Load existing functions from metadata files
        
        // Initialize metrics database
        let metrics_db = Arc::new(DashMap::new());
        
        Ok(Self {
            functions_dir,
            functions_db,
            metrics_db,
            auth_validator: Arc::new(Mutex::new(Box::new(auth_validator))),
        })
    }
    
    /// Validate GitHub authentication token
    async fn validate_auth(&self, username: &str, token: &str) -> anyhow::Result<bool> {
        let validator = self.auth_validator.lock().await;
        validator(username, token)
    }
    
    /// Extract username from GitHub token
    async fn get_username_from_token(&self, token: &str) -> FunctionResult<String> {
        // This is a placeholder. In a real implementation, you would
        // make a request to the GitHub API to get the username.
        // For now, we'll assume the token is in the format "username:token"
        let parts: Vec<&str> = token.split(':').collect();
        if parts.len() != 2 {
            return Err(FunctionError::AuthError("Invalid token format".to_string()));
        }
        Ok(parts[0].to_string())
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
        if !self.validate_auth(&username, &github_auth_token).await.map_err(|e| 
            FunctionError::AuthError(e.to_string()))? {
            return Err(FunctionError::AuthError("Invalid GitHub authentication token".to_string()));
        }
        
        // Check if function name is valid
        if name.is_empty() || !name.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-') {
            return Err(FunctionError::InvalidInput(
                "Invalid function name. Use only alphanumeric characters, underscores, and hyphens.".to_string()
            ));
        }
        
        // Check if function already exists
        if self.functions_db.contains_key(&name) {
            let existing = self.functions_db.get(&name).unwrap();
            if existing.owner != username {
                return Err(FunctionError::PermissionDenied(
                    "A function with this name already exists and belongs to another user".to_string()
                ));
            }
        }
        
        // Save the WASM file
        let wasm_path = self.functions_dir.join(format!("{}.wasm", name));
        let mut file = fs::File::create(&wasm_path).map_err(|e| 
            FunctionError::InternalError(format!("Failed to create file: {}", e)))?;
        file.write_all(&wasm_file).map_err(|e| 
            FunctionError::InternalError(format!("Failed to write file: {}", e)))?;
        
        // Create function info
        let now = chrono::Utc::now().to_rfc3339();
        let function_info = FunctionInfo {
            name: name.clone(),
            owner: username,
            published_at: now,
            usage: format!("https://faasta.xyz/{}", name),
        };
        
        // Save function metadata
        self.functions_db.insert(name.clone(), function_info.clone());
        
        // TODO: Save metadata to a file or database
        
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
        if !self.validate_auth(&username, &github_auth_token).await.map_err(|e| 
            FunctionError::AuthError(e.to_string()))? {
            return Err(FunctionError::AuthError("Invalid GitHub authentication token".to_string()));
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
        if !self.validate_auth(&username, &github_auth_token).await.map_err(|e| 
            FunctionError::AuthError(e.to_string()))? {
            return Err(FunctionError::AuthError("Invalid GitHub authentication token".to_string()));
        }
        
        // Check if function exists
        if let Some(entry) = self.functions_db.get(&name) {
            // Check if user owns the function
            if entry.owner != username {
                return Err(FunctionError::PermissionDenied(
                    "You don't have permission to unpublish this function".to_string()
                ));
            }
            
            // Remove function from database
            self.functions_db.remove(&name);
            
            // Remove WASM file
            let wasm_path = self.functions_dir.join(format!("{}.wasm", name));
            if wasm_path.exists() {
                fs::remove_file(wasm_path).map_err(|e| 
                    FunctionError::InternalError(format!("Failed to remove file: {}", e)))?;
            }
            
            // TODO: Remove metadata file
            
            Ok(())
        } else {
            Err(FunctionError::NotFound(format!("Function '{}' not found", name)))
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
        if !self.validate_auth(&username, &github_auth_token).await.map_err(|e|
            FunctionError::AuthError(e.to_string()))? {
            return Err(FunctionError::AuthError("Invalid GitHub authentication token".to_string()));
        }
        
        // Collect metrics from all functions
        let mut function_metrics = Vec::new();
        let mut total_time = 0;
        let mut total_calls = 0;
        
        for entry in self.metrics_db.iter() {
            let function_name = entry.key().clone();
            let (time, calls, last_called) = *entry.value();
            
            // Convert timestamp to ISO string
            let _last_called_time = UNIX_EPOCH + Duration::from_millis(last_called);
            let last_called_str = chrono::Utc::now().to_rfc3339(); // Placeholder, should use actual timestamp
            
            function_metrics.push(FunctionMetricsResponse {
                function_name,
                total_time_millis: time,
                call_count: calls,
                last_called: last_called_str,
            });
            
            total_time += time;
            total_calls += calls;
        }
        
        Ok(Metrics {
            total_time,
            total_calls,
            function_metrics,
        })
    }
}

/// Helper function to create a service implementation with GitHub auth
pub fn create_service_with_github_auth(
    functions_dir: PathBuf,
    github_auth: Arc<impl Fn(&str, &str) -> anyhow::Result<bool> + Send + Sync + 'static>,
) -> anyhow::Result<FunctionServiceImpl> {
    FunctionServiceImpl::new(functions_dir, move |username, token| github_auth(username, token))
}