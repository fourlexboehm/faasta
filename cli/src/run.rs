use anyhow::{Result, anyhow};
use bitrpc::{RpcError, cyper::CyperTransport};
use faasta_interface::{FunctionResult, FunctionServiceRpcClient};
use std::io;
use std::path::{Path as StdPath, PathBuf};
use std::process::exit;
use tracing::debug;
use url::Url;

/// Compare two file paths in a slightly more robust way.
/// (On Windows, e.g., backslash vs forward slash).
fn same_file_path(a: &str, b: &str) -> bool {
    // Convert both to a canonical PathBuf
    let path_a = StdPath::new(a).components().collect::<Vec<_>>();
    let path_b = StdPath::new(b).components().collect::<Vec<_>>();
    path_a == path_b
}

#[derive(Clone)]
pub struct FunctionServiceClient {
    endpoint: String,
}

impl FunctionServiceClient {
    fn new(endpoint: String) -> Self {
        Self { endpoint }
    }

    fn new_transport(&self) -> CyperTransport {
        CyperTransport::new(self.endpoint.clone())
    }

    pub async fn publish(
        &self,
        wasm_file: Vec<u8>,
        name: String,
        github_auth_token: String,
    ) -> Result<FunctionResult<String>, RpcError> {
        let mut client = FunctionServiceRpcClient::new(self.new_transport());
        let response = client.publish(wasm_file, name, github_auth_token).await?;
        Ok(response)
    }

    pub async fn list_functions(
        &self,
        github_auth_token: String,
    ) -> Result<FunctionResult<Vec<faasta_interface::FunctionInfo>>, RpcError> {
        let mut client = FunctionServiceRpcClient::new(self.new_transport());
        let response = client.list_functions(github_auth_token).await?;
        Ok(response)
    }

    pub async fn unpublish(
        &self,
        name: String,
        github_auth_token: String,
    ) -> Result<FunctionResult<()>, RpcError> {
        let mut client = FunctionServiceRpcClient::new(self.new_transport());
        let response = client.unpublish(name, github_auth_token).await?;
        Ok(response)
    }

    pub async fn get_metrics(
        &self,
        github_auth_token: String,
    ) -> Result<FunctionResult<faasta_interface::Metrics>, RpcError> {
        let mut client = FunctionServiceRpcClient::new(self.new_transport());
        let response = client.get_metrics(github_auth_token).await?;
        Ok(response)
    }
}

fn normalize_endpoint(server_addr: &str) -> Result<String> {
    let trimmed = server_addr.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("Server address cannot be empty"));
    }

    let mut url = if trimmed.contains("://") {
        Url::parse(trimmed).map_err(|e| anyhow!("Invalid server address '{trimmed}': {e}"))?
    } else {
        Url::parse(&format!("https://{trimmed}"))
            .or_else(|_| Url::parse(&format!("https://{trimmed}/")))
            .map_err(|e| anyhow!("Invalid server address '{trimmed}': {e}"))?
    };

    if url.scheme() != "https" {
        url.set_scheme("https")
            .map_err(|_| anyhow!("Server address must use HTTPS"))?;
    }

    if url.path() == "/" {
        url.set_path("/rpc");
    }

    Ok(url.to_string())
}

// Create a connection to the function service
pub async fn connect_to_function_service(server_addr: &str) -> Result<FunctionServiceClient> {
    let endpoint = normalize_endpoint(server_addr)?;
    debug!("Configured RPC endpoint: {}", endpoint);
    Ok(FunctionServiceClient::new(endpoint))
}

/// Get the target directory and package name for the current project
pub fn get_project_info() -> Result<(PathBuf, String, PathBuf), io::Error> {
    let spinner = indicatif::ProgressBar::new_spinner();
    spinner.set_message("Getting project information...");
    spinner.enable_steady_tick(std::time::Duration::from_millis(100));

    // Get package info using cargo metadata
    let output = std::process::Command::new("cargo")
        .args(["metadata", "--format-version=1"])
        .output()
        .unwrap_or_else(|e| {
            spinner.finish_and_clear();
            eprintln!("Failed to run cargo metadata: {e}");
            exit(1);
        });

    if !output.status.success() {
        spinner.finish_and_clear();
        eprintln!("Failed to retrieve cargo metadata");
        exit(1);
    }

    // Parse JSON
    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
        spinner.finish_and_clear();
        eprintln!("Failed to parse cargo metadata: {e}");
        exit(1);
    });

    // Extract target_directory
    let target_directory = metadata
        .get("target_directory")
        .and_then(serde_json::Value::as_str)
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            spinner.finish_and_clear();
            eprintln!("No 'target_directory' found in cargo metadata");
            exit(1);
        });

    // Get the package name from the current directory's Cargo.toml
    let packages = metadata
        .get("packages")
        .and_then(serde_json::Value::as_array)
        .unwrap_or_else(|| {
            spinner.finish_and_clear();
            eprintln!("No 'packages' found in cargo metadata");
            exit(1);
        });

    // Find the package for the current directory
    let current_dir = std::env::current_dir().unwrap_or_else(|e| {
        spinner.finish_and_clear();
        eprintln!("Failed to get current directory: {e}");
        exit(1);
    });

    let package_name = packages
        .iter()
        .filter_map(|pkg| {
            let manifest_path = pkg.get("manifest_path")?.as_str()?;
            let pkg_dir = StdPath::new(manifest_path).parent()?;
            if same_file_path(&pkg_dir.to_string_lossy(), &current_dir.to_string_lossy()) {
                pkg.get("name")?.as_str().map(String::from)
            } else {
                None
            }
        })
        .next()
        .unwrap_or_else(|| {
            spinner.finish_and_clear();
            eprintln!("Could not find package for current directory");
            exit(1);
        });

    spinner.finish_and_clear();
    Ok((target_directory, package_name, current_dir))
}

/// Build the project for wasm32-wasip2 target
pub fn build_project(package_root: &PathBuf) -> Result<(), io::Error> {
    let spinner = indicatif::ProgressBar::new_spinner();
    spinner.set_message("Building optimized x86_64 binary...");
    spinner.enable_steady_tick(std::time::Duration::from_millis(100));

    // Validate the project structure
    if !package_root.join("src").join("lib.rs").exists() {
        spinner.finish_and_clear();
        eprintln!("Error: src/lib.rs is missing. This file is required for Faasta functions.");
        eprintln!("Hint: Run 'cargo faasta new <n>' to create a new Faasta project.");
        exit(1);
    }

    let status = std::process::Command::new("cargo")
        .args(["zigbuild", "--release", "--target", "x86_64-unknown-linux-gnu"])
        .current_dir(package_root)
        .status()
        .unwrap_or_else(|e| {
            spinner.finish_and_clear();
            eprintln!("Failed to run cargo zigbuild, did you install it?: {e}");
            let _status = std::process::Command::new("cargo")
                .args(["binstall", "cargo-zigbuild"]).status().unwrap_or_else(|e| {
                eprintln!("Failed to run cargo binstall, did you install it?: {e}");
                std::process::Command::new("cargo")
                .args(["install", "cargo-zigbuild"]).status().unwrap_or_else(|_e| std::process::exit(1))
            });
            exit(1);
        });

    if !status.success() {
        spinner.finish_and_clear();
        eprintln!("Build failed");
        exit(1);
    }

    spinner.finish_and_clear();
    println!("✅ Build successful!");
    Ok(())
}

// The function to handle the run command
pub async fn handle_run(port: u16) -> io::Result<()> {
    // Get project information
    let (target_directory, package_name, package_root) = get_project_info()?;

    // Display project info
    println!("Building project: {package_name}");
    println!("Project root: {}", package_root.display());

    // Build the project first
    build_project(&package_root)?;

    // Get the full WASM file path - use same logic as in deploy
    let rust_compiled_name = package_name.replace('-', "_");
    let wasm_filename = format!("{rust_compiled_name}.wasm");
    let wasm_path = target_directory
        .join("wasm32-wasip2")
        .join("release")
        .join(wasm_filename);

    // Ensure the WASM file exists
    if !wasm_path.exists() {
        eprintln!(
            "Error: Could not find compiled WASM at: {}",
            wasm_path.display()
        );
        eprintln!("Build seems to have failed or produced output in a different location.");
        exit(1);
    }

    println!("Starting local server on port {port}...");
    let status = std::process::Command::new("wasmtime")
        .args(["serve", &wasm_path.to_string_lossy()])
        .current_dir(&package_root)
        .status()
        .unwrap_or_else(|e| {
            eprintln!("Failed to run wasmtime serve: {e}");
            exit(1);
        });

    if !status.success() {
        eprintln!("wasmtime serve exited with an error");
        exit(1);
    }

    Ok(())
}
