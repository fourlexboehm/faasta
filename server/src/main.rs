mod build_tooling;
mod github_auth;
mod metrics;

use crate::build_tooling::{generate_hmac, handle_upload_and_build};
use crate::github_auth::GitHubAuth;
use std::cmp::max;
use std::env;
use std::sync::Arc;

use axum::body::Body;
use axum::error_handling::HandleErrorLayer;
use axum::extract::Path;
use axum::extract::State;
use axum::response::Response;
use axum::{
    body::Bytes,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    BoxError, Router,
};
use cap_async_std::fs::Dir;
use dashmap::DashMap;
use http::{HeaderMap, Method, Uri};
use lazy_static::lazy_static;
use libloading::{Library, Symbol};
use std::error::Error;
use std::fmt;
use std::fmt::{Display, Formatter};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use cap_async_std::ambient_authority;
use tokio::fs;
use tower::timeout::TimeoutLayer;
use tower::{timeout, ServiceBuilder};
use tower_http::catch_panic::CatchPanicLayer;

// Type for function handling requests
type HandleRequestFn =
extern "Rust" fn(
    Method,
    Uri,
    HeaderMap,
    Bytes,
    Dir,
) -> Pin<Box<dyn Future<Output=Response<Body>> + Send + 'static>>;

// Cache for loaded libraries
lazy_static! {
    static ref LIB_CACHE: DashMap<String, LoadedFunction> = DashMap::new();
}

// Structure to track loaded functions and their usage
struct LoadedFunction {
    handle_fn: HandleRequestFn, // the symbol as a raw function pointer
    usage_count: AtomicUsize,
}

impl LoadedFunction {
    fn new(handle_fn: HandleRequestFn) -> Self {
        Self {
            handle_fn,
            usage_count: AtomicUsize::new(0),
        }
    }
}

// Application state
#[derive(Clone)]
struct AppState {
    github_auth: Arc<GitHubAuth>,
}

// Handle function invocation
async fn handle_invoke_rs(
    State(_state): State<AppState>,
    Path(function_name): Path<String>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response<Body> {
    // attempt to fetch from the cache
    let loaded_fn = match LIB_CACHE.get(&function_name) {
        Some(loaded) => loaded,
        None => {
            // Otherwise, open it
            let path = format!("./functions/{name}", name = function_name);
            if fs::try_exists(&path).await.is_err() {
                return (StatusCode::NOT_FOUND, "Function not found").into_response();
            }
            let new_lib = unsafe {
                match Library::new(&path) {
                    Ok(lib) => lib,
                    Err(_) => {
                        return (StatusCode::NOT_FOUND, "Function could not be loaded").into_response()
                    }
                }
            };

            // Generate the symbol name (e.g. "dy_...")
            let secret = env::var("FAASTA_HMAC_SECRET").unwrap_or_else(|_| "faasta-dev-secret-key".to_string());
            let hmac = "dy_".to_string() + &*generate_hmac(&*function_name, &secret);
            
            // Note: GitHub auth is only needed for upload, not for invoke

            // Safely look up the symbol *once*
            let symbol: Symbol<HandleRequestFn> = unsafe {
                match new_lib.get(hmac.as_bytes()) {
                    Ok(s) => s,
                    Err(_) => {
                        return (StatusCode::NOT_FOUND, "Function handler not found").into_response();
                    }
                }
            };

            // Turn the Symbol<HandleRequestFn> into a raw fn pointer
            let handle_fn = *symbol;

            // Store in the map
            let inserted = LoadedFunction::new(handle_fn);
            LIB_CACHE.insert(function_name.clone(), inserted);

            // get a fresh reference from the map
            LIB_CACHE.get(&function_name).unwrap()
        }
    };
    let start_time = std::time::Instant::now();
    loaded_fn.usage_count.fetch_add(1, Ordering::Relaxed); // or track usage times, etc.

    // Prepare your sandbox if needed
    let path = format!("./sandbox/{function_name}");
    if !fs::try_exists(&path).await.unwrap() {
        fs::create_dir_all(&path).await.unwrap();
    }
    let sandbox = Dir::open_ambient_dir(&path, ambient_authority()).await.unwrap();

    // Then call the function pointer directly
    let response = (loaded_fn.handle_fn)(method, uri, headers, body, sandbox).await;

    // Optionally track timings
    loaded_fn
        .usage_count
        .fetch_add(max(start_time.elapsed().as_millis() as usize, 1), Ordering::Relaxed);

    println!(
        "Function {} took {:?}",
        function_name,
        start_time.elapsed()
    );
    if LIB_CACHE.len() > 1000 {
        tokio::spawn(async move {
            // Remove the least used function
            let min_func = LIB_CACHE
                .iter()
                .min_by_key(|it| it.value().usage_count.load(std::sync::atomic::Ordering::Relaxed));
            if let Some(min_func) = min_func {
                LIB_CACHE.remove(min_func.key());
            }
        });
    }
    response
}

// Modified upload handler to integrate with GitHub auth
async fn handle_upload_with_auth(
    State(state): State<AppState>,
    path: Path<String>,
    headers: HeaderMap,
    multipart: axum::extract::Multipart,
) -> impl IntoResponse {
    // Check GitHub authentication
    let github_username = match headers.get("X-GitHub-Username").and_then(|h| h.to_str().ok()) {
        Some(username) => username,
        None => return (StatusCode::UNAUTHORIZED, "GitHub username required").into_response(),
    };
    
    let auth_token = match headers.get("Authorization").and_then(|h| h.to_str().ok()) {
        Some(token) => token,
        None => return (StatusCode::UNAUTHORIZED, "Authorization header required").into_response(),
    };
    
    // Validate the GitHub authentication using OAuth token
    match state.github_auth.validate_oauth_token(github_username, auth_token).await {
        Ok(true) => {},
        Ok(false) => return (StatusCode::UNAUTHORIZED, "Invalid GitHub authentication").into_response(),
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "Failed to validate GitHub authentication").into_response(),
    };
    
    // Check if user can upload more projects
    let function_name = path.0;
    if !state.github_auth.can_upload_project(github_username, &function_name) {
        return (
            StatusCode::FORBIDDEN, 
            format!("You have reached the maximum limit of {} projects", 10)
        ).into_response();
    }
    
    // Generate HMAC for function validation
    let secret = env::var("FAASTA_HMAC_SECRET").unwrap_or_else(|_| "faasta-dev-secret-key".to_string());
    let hmac = generate_hmac(&function_name, &secret);
    
    // Process the upload and get the response
    let build_result = handle_upload_and_build(Path(function_name.clone()), multipart).await;
    
    // Convert to response to check the status
    let response = build_result.into_response();
    
    // Only register the project if the build was successful
    if response.status() == StatusCode::OK {
        let _ = state.github_auth.add_project(github_username, &function_name, &hmac).await;
    }
    
    // Return the build result (convert it to the correct response type)
    response
}

// Error handler for timeouts
#[derive(Debug)]
struct TimeoutError {
    message: String,
}

impl TimeoutError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl Display for TimeoutError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl Error for TimeoutError {}

impl IntoResponse for TimeoutError {
    fn into_response(self) -> Response {
        (StatusCode::GATEWAY_TIMEOUT, self.message).into_response()
    }
}

// This is our handler for timeouts (or other errors) produced by `TimeoutLayer`
async fn handle_timeout_error(error: BoxError) -> TimeoutError {
    if error.is::<timeout::error::Elapsed>() {
        TimeoutError::new("Request timed out")
    } else {
        TimeoutError::new(format!("Unhandled error: {}", error))
    }
}

#[tokio::main]
async fn main() {
    // Initialize GitHub App authentication
    println!("Initializing GitHub App authentication...");
   
    // Create GitHub auth instance
    let github_auth = match GitHubAuth::new().await {
        Ok(auth) => {
            println!("GitHub App authentication initialized successfully");
            Arc::new(auth)
        },
        Err(e) => {
            eprintln!("Failed to initialize GitHub App authentication: {}", e);
            eprintln!("Continuing without GitHub authentication");
            // Create a default instance for development
            Arc::new(GitHubAuth::new().await.unwrap())
        }
    };
    
    // Create the app state
    let state = AppState {
        github_auth,
    };
    
    // Setup the service middleware
    let service = ServiceBuilder::new()
        .layer(CatchPanicLayer::new())
        .layer(HandleErrorLayer::new(handle_timeout_error))
        .layer(TimeoutLayer::new(Duration::from_secs(900)));
    
    // Setup routes
    let app = Router::new()
        .route("/metrics", get(metrics::get_metrics))
        .route("/upload/{function_name}", post(handle_upload_with_auth))
        .route(
            "/{function_name}",
            get(handle_invoke_rs).post(handle_invoke_rs),
        )
        .layer(service)
        .with_state(state);

    // Start the server
    println!("Starting server on 0.0.0.0:8080...");
    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await.unwrap();
    axum::serve(listener, app.into_make_service())
        .await
        .unwrap();
}
