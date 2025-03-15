use std::io;
use axum::{
    extract::{Path, State},
    routing::get,
    Router, 
    response::IntoResponse,
    body::Bytes,
    http::{HeaderMap, Method, StatusCode, Uri},
};
use cap_async_std::{fs::Dir, ambient_authority};
use tokio::net::TcpListener;
use faasta_analyze::{lint_project, build_project};
use libloading::{Library, Symbol};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::exit;
use std::fs;

// The function to handle the run command
pub async fn handle_run(port: u16) -> io::Result<()> {
    let spinner = indicatif::ProgressBar::new_spinner();
    spinner.set_message("Building project...");
    spinner.enable_steady_tick(std::time::Duration::from_millis(100));

    let (package_root, package_name) = crate::find_root_package().expect("Failed to find root package");

    // Display project info
    println!("Building project: {}", package_name);
    println!("Project root: {}", package_root.display());
    
    // Validate the project structure
    if !package_root.join("src").join("lib.rs").exists() {
        spinner.finish_and_clear();
        eprintln!("Error: src/lib.rs is missing. This file is required for FaaSta functions.");
        eprintln!("Hint: Run 'cargo faasta new <name>' to create a new FaaSta project.");
        exit(1);
    }

    // Run safety lints
    spinner.set_message("Running security and safety checks...");
    lint_project(&package_root).await.unwrap_or_else(|e| {
        spinner.finish_and_clear();
        eprintln!("Failed security checks: {}", e);
        eprintln!("Please fix the security issues and try again.");
        exit(1);
    });

    // Build the project
    spinner.set_message("Building optimized release binary...");
    build_project(&package_root).await.unwrap_or_else(|e| {
        spinner.finish_and_clear();
        eprintln!("Build failed: {}", e);
        exit(1);
    });

    spinner.finish_and_clear();
    println!("✅ Build successful!");
    
    // Run the function locally
    println!("Starting local function server on port {}...", port);
    
    // Create the socket address for the server
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    println!("Function URL: http://localhost:{}/{}", port, package_name);
    println!("Test endpoint: http://localhost:{}/{}/hello", port, package_name);
    println!("Press Ctrl+C to stop the server");
    
    // Get the library extension based on the platform
    let extension = if cfg!(target_os = "linux") {
        "so"
    } else if cfg!(target_os = "macos") {
        "dylib"
    } else if cfg!(target_os = "windows") {
        "dll"
    } else {
        "so"
    };
    
    // Path to the compiled library
    let library_path = package_root
        .join("target")
        .join("release")
        .join(format!("lib{}.{}", package_name, extension));
    
    if !library_path.exists() {
        eprintln!("Error: Could not find compiled library at: {}", library_path.display());
        exit(1);
    }
    
    println!("Loading function from: {}", library_path.display());
    
    // Create sandbox directory for the function
    let sandbox_dir = package_root.join("sandbox");
    if !sandbox_dir.exists() {
        fs::create_dir_all(&sandbox_dir).expect("Failed to create sandbox directory");
    }

    // Create app state with function info
    let state = AppState {
        package_name,
        library_path,
        sandbox_path: sandbox_dir,
    };

    // Set up router with our shared state
    let app = Router::new()
        .route("/:function_name/*path", get(handle_request).post(handle_request))
        .with_state(state);

    // Start the server
    let listener = TcpListener::bind(addr).await?;
    println!("Server started successfully, listening on {}", addr);
    
    // Run the server
    axum::serve(listener, app).await?;

    Ok(())
}

// Shared state for our app
#[derive(Clone)]
struct AppState {
    package_name: String,
    library_path: PathBuf,
    sandbox_path: PathBuf,
}

// Function handler
async fn handle_request(
    State(state): State<AppState>,
    Path((function_name, path)): Path<(String, String)>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    // Check if the function name matches what we're expecting
    if function_name != state.package_name {
        return (
            StatusCode::NOT_FOUND,
            format!("Function '{}' not found", function_name),
        ).into_response();
    }
    
    // Construct a URI with the path
    let path_str = format!("/{}", path);
    let uri_string = path_str.clone();
    let uri = Uri::try_from(uri_string).unwrap_or(uri);
    
    // Open the library
    let lib = unsafe {
        match Library::new(&state.library_path) {
            Ok(lib) => lib,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Failed to load library: {}", e),
                ).into_response();
            }
        }
    };
    
    // Generate the symbol name
    let symbol_name = "dy_".to_string() + &*function_name;
    
    // Define the handler function type
    type HandlerFn = fn(
        Method,
        Uri,
        HeaderMap, 
        Bytes,
        Dir,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = axum::response::Response<axum::body::Body>> + Send + 'static>>;
    
    // Look up the handler function
    let handler_fn: Symbol<HandlerFn> = unsafe {
        match lib.get(symbol_name.as_bytes()) {
            Ok(symbol) => symbol,
            Err(e) => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Failed to find handler function: {}", e),
                ).into_response();
            }
        }
    };
    
    // Open the sandbox directory
    match Dir::open_ambient_dir(&state.sandbox_path, ambient_authority()).await {
        Ok(sandbox) => {
            // Call the handler function
            (handler_fn)(method, uri, headers, body, sandbox).await
        },
        Err(e) => {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to open sandbox directory: {}", e),
            ).into_response()
        }
    }
}