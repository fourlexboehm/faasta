use bitrpc::bitcode::{Decode, Encode};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const MAX_WASM_SIZE: usize = 30 * 1024 * 1024;

// Define a custom error type that can be serialized
#[derive(Debug, Error, Serialize, Deserialize, Clone, Encode, Decode)]
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
#[derive(Clone, Debug, Serialize, Deserialize, Encode, Decode, bincode::Encode, bincode::Decode)]
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
#[derive(Clone, Debug, Serialize, Deserialize, Encode, Decode)]
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
#[derive(Clone, Debug, Serialize, Deserialize, Encode, Decode)]
pub struct Metrics {
    /// Total execution time across all functions in milliseconds
    pub total_time: u64,
    /// Total number of function calls
    pub total_calls: u64,
    /// Metrics for individual functions
    pub function_metrics: Vec<FunctionMetricsResponse>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Encode, Decode)]
pub struct PublishRequest {
    pub wasm_file: Vec<u8>,
    pub name: String,
    pub github_auth_token: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Encode, Decode)]
pub struct PublishResponse {
    pub result: FunctionResult<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Encode, Decode)]
pub struct ListFunctionsRequest {
    pub github_auth_token: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Encode, Decode)]
pub struct ListFunctionsResponse {
    pub result: FunctionResult<Vec<FunctionInfo>>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Encode, Decode)]
pub struct UnpublishRequest {
    pub name: String,
    pub github_auth_token: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Encode, Decode)]
pub struct UnpublishResponse {
    pub result: FunctionResult<()>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Encode, Decode)]
pub struct GetMetricsRequest {
    pub github_auth_token: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Encode, Decode)]
pub struct GetMetricsResponse {
    pub result: FunctionResult<Metrics>,
}

/// Service interface for managing functions via bitrpc.
#[bitrpc::service]
#[derive(Clone, Debug, Encode, Decode)]
pub enum FunctionServiceRpc {
    /// Publish a new function
    Publish(PublishRequest, PublishResponse),
    /// List all functions for the authenticated user
    ListFunctions(ListFunctionsRequest, ListFunctionsResponse),
    /// Unpublish a function
    Unpublish(UnpublishRequest, UnpublishResponse),
    /// Get metrics for all functions
    GetMetrics(GetMetricsRequest, GetMetricsResponse),
}

pub use FunctionServiceRpcResponse as FunctionServiceResponse;
pub use FunctionServiceRpcService as FunctionService;
