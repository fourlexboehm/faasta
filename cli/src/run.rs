use anyhow::{anyhow, Context, Result};
use faasta_interface::FunctionServiceClient;
use std::io;
// futures prelude removed
use s2n_quic::client::Connect;
use s2n_quic::Client;
use std::fs;
use std::net::SocketAddr;
use std::path::{Path as StdPath, PathBuf};
use std::process::exit;
use tarpc::serde_transport as transport;
use tarpc::tokio_serde::formats::Bincode;
use tarpc::tokio_util::codec::LengthDelimitedCodec;
use tracing::debug;

/// Compare two file paths in a slightly more robust way.
/// (On Windows, e.g., backslash vs forward slash).
fn same_file_path(a: &str, b: &str) -> bool {
    // Convert both to a canonical PathBuf
    let path_a = StdPath::new(a).components().collect::<Vec<_>>();
    let path_b = StdPath::new(b).components().collect::<Vec<_>>();
    path_a == path_b
}

// Create a connection to the function service
pub async fn connect_to_function_service(server_addr: &str) -> Result<FunctionServiceClient> {
    // Set up the QUIC client with minimal logging
    let client = Client::builder()
        .with_io("0.0.0.0:0")
        .context("Failed to set up client IO")?
        .start()
        .context("Failed to start client")?;

    // Parse the server address, handling both IP:port and hostname:port formats
    let addr: SocketAddr = match server_addr.parse() {
        Ok(addr) => addr,
        Err(_) => {
            // Try to resolve the hostname
            let parts: Vec<&str> = server_addr.split(':').collect();
            if parts.len() != 2 {
                return Err(anyhow!(
                    "Invalid server address format. Expected hostname:port or IP:port"
                ));
            }

            let hostname = parts[0];
            let port = parts[1].parse::<u16>().context("Invalid port number")?;

            // For localhost, use 127.0.0.1
            if hostname == "localhost" || hostname == "localhost.localdomain" {
                format!("127.0.0.1:{}", port)
                    .parse()
                    .context("Failed to parse localhost address")?
            } else {
                // For other hostnames, try to resolve using DNS
                match tokio::net::lookup_host(format!("{}:{}", hostname, port)).await {
                    Ok(mut addrs) => {
                        // Take the first resolved address
                        if let Some(addr) = addrs.next() {
                            addr
                        } else {
                            return Err(anyhow!(
                                "Could not resolve hostname: {}. No addresses found.",
                                hostname
                            ));
                        }
                    }
                    Err(e) => {
                        return Err(anyhow!(
                            "Could not resolve hostname: {}. Error: {}",
                            hostname,
                            e
                        ));
                    }
                }
            }
        }
    };

    let server_name = if server_addr.starts_with("localhost:")
        || server_addr.contains("localhost.localdomain:")
    {
        "localhost".to_string()
    } else {
        // Extract the hostname from the original server_addr string for SNI
        let parts: Vec<&str> = server_addr.split(':').collect();
        parts[0].to_string()
    };

    let connect = Connect::new(addr).with_server_name(server_name.as_str());

    let mut connection = client
        .connect(connect)
        .await
        .map_err(|e| {
            // Provide minimal error info for handshake failures
            if e.to_string().contains("handshake") {
                if e.to_string().contains("timeout") {
                    anyhow!("Failed to connect: Handshake timeout. Check your network connection or firewall settings.")
                } else {
                    anyhow!("Failed to connect: TLS handshake error. The server may be down or unreachable.")
                }
            } else {
                anyhow!("Failed to connect: {}", e)
            }
        })?;

    // Open bidirectional stream
    let stream = connection
        .open_bidirectional_stream()
        .await
        .map_err(|e| anyhow!("Failed to open stream: {}", e))?;
    debug!("Opened bidirectional stream to function service");

    let framed = LengthDelimitedCodec::builder().new_framed(stream);
    let transport = transport::new(framed, Bincode::default());
    let client = FunctionServiceClient::new(Default::default(), transport).spawn();

    Ok(client)
}

// The function to handle the run command
pub async fn handle_run(port: u16) -> io::Result<()> {
    let spinner = indicatif::ProgressBar::new_spinner();
    spinner.set_message("Building project...");
    spinner.enable_steady_tick(std::time::Duration::from_millis(100));
    // Get package info using cargo metadata
    let output = std::process::Command::new("cargo")
        .args(["metadata", "--format-version=1"])
        .output()
        .unwrap_or_else(|e| {
            spinner.finish_and_clear();
            eprintln!("Failed to run cargo metadata: {}", e);
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
        eprintln!("Failed to parse cargo metadata: {}", e);
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
        eprintln!("Failed to get current directory: {}", e);
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

    let package_root = current_dir;

    // Display project info
    println!("Building project: {}", package_name);
    println!("Project root: {}", package_root.display());

    // Validate the project structure
    if !package_root.join("src").join("lib.rs").exists() {
        spinner.finish_and_clear();
        eprintln!("Error: src/lib.rs is missing. This file is required for FaaSta functions.");
        eprintln!("Hint: Run 'cargo faasta new <n>' to create a new FaaSta project.");
        exit(1);
    }

    // Run safety lints - removed (analyze crate no longer used)

    // Build the project for wasm32-wasip2 target
    spinner.set_message("Building optimized WASI component...");

    // Build with wasm32-wasip2 target
    let status = std::process::Command::new("cargo")
        .args(["build", "--release", "--target", "wasm32-wasip2"])
        .current_dir(&package_root)
        .status()
        .unwrap_or_else(|e| {
            spinner.finish_and_clear();
            eprintln!("Failed to run cargo build: {}", e);
            exit(1);
        });

    if !status.success() {
        spinner.finish_and_clear();
        eprintln!("Build failed");
        exit(1);
    }

    // Convert to component using wasm-tools
    spinner.set_message("Converting to WASI component...");

    // Convert hyphens to underscores in package name for the WASM file
    let wasm_filename = format!("{}.wasm", package_name.replace('-', "_"));
    let wasm_path = target_directory
        .join("wasm32-wasip2")
        .join("release")
        .join(wasm_filename);

    // Convert hyphens to underscores in package name for the component file
    let component_filename = format!("{}_component.wasm", package_name.replace('-', "_"));
    let component_path = target_directory
        .join("wasm32-wasip2")
        .join("release")
        .join(component_filename);

    let status = std::process::Command::new("wasm-tools")
        .args([
            "component",
            "new",
            wasm_path.to_str().unwrap(),
            "-o",
            component_path.to_str().unwrap(),
        ])
        .current_dir(&package_root)
        .status()
        .unwrap_or_else(|e| {
            spinner.finish_and_clear();
            eprintln!("Failed to run wasm-tools: {}", e);
            exit(1);
        });

    if !status.success() {
        spinner.finish_and_clear();
        eprintln!("Component conversion failed");
        exit(1);
    }

    spinner.finish_and_clear();
    println!("✅ Build successful!");

    // Run the function locally using wasmtime serve
    println!("Starting local function server on port {}...", port);
    println!("Function URL: http://localhost:{}", port);
    println!("Press Ctrl+C to stop the server");

    // Path to the compiled WASI component
    // Convert hyphens to underscores in package name for the component file
    let component_filename = format!("{}_component.wasm", package_name.replace('-', "_"));
    let component_path = target_directory
        .join("wasm32-wasip2")
        .join("release")
        .join(component_filename);

    if !component_path.exists() {
        eprintln!(
            "Error: Could not find compiled WASI component at: {}",
            component_path.display()
        );
        exit(1);
    }

    println!("Loading function from: {}", component_path.display());

    // Copy the component to the server's functions directory for deployment
    let server_functions_dir = PathBuf::from("server-wasi/functions");
    if !server_functions_dir.exists() {
        fs::create_dir_all(&server_functions_dir)
            .expect("Failed to create server functions directory");
    }

    // Use the original package name for the server function path (server handles the conversion)
    let server_function_path = server_functions_dir.join(format!("{}.wasm", package_name));
    fs::copy(&component_path, &server_function_path).unwrap_or_else(|e| {
        eprintln!(
            "Failed to copy component to server functions directory: {}",
            e
        );
        exit(1);
    });

    println!("Deployed function to: {}", server_function_path.display());
    println!("Running function with wasmtime serve...");

    // Run wasmtime serve with the component
    let status = std::process::Command::new("wasmtime")
        .args([
            "serve",
            "--http-port",
            &port.to_string(),
            "--addr",
            "127.0.0.1",
            component_path.to_str().unwrap(),
        ])
        .current_dir(&package_root)
        .status()
        .unwrap_or_else(|e| {
            eprintln!("Failed to run wasmtime serve: {}", e);
            exit(1);
        });

    if !status.success() {
        eprintln!("wasmtime serve exited with an error");
        exit(1);
    }

    Ok(())
}
