mod github_oauth;
mod init;
mod run;

use anyhow::Error;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
// Removed unused imports
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::{exit, Command};
// Removed unused imports

const INVOKE_URL: &str = "https://faasta.xyz/";
const MAX_PROJECTS_PER_USER: usize = 10;
const CONFIG_DIR: &str = ".faasta";
const CONFIG_FILE: &str = "config.json";

#[derive(Debug)]
enum CustomError {
    Io(std::io::Error),
    Reqwest(reqwest::Error),
}

impl From<std::io::Error> for CustomError {
    fn from(err: std::io::Error) -> CustomError {
        CustomError::Io(err)
    }
}

impl From<reqwest::Error> for CustomError {
    fn from(err: reqwest::Error) -> CustomError {
        CustomError::Reqwest(err)
    }
}

impl fmt::Display for CustomError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            CustomError::Io(err) => write!(f, "IO error: {}", err),
            CustomError::Reqwest(err) => write!(f, "Reqwest error: {}", err),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
struct FaastaConfig {
    github_username: Option<String>,
    github_token: Option<String>,
}

/// Get the configuration directory
fn get_config_dir() -> PathBuf {
    let home_dir = dirs::home_dir().expect("Could not find home directory");
    home_dir.join(CONFIG_DIR)
}

/// Load the config file or create a new one
fn load_config() -> Result<FaastaConfig, Error> {
    let config_dir = get_config_dir();
    let config_path = config_dir.join(CONFIG_FILE);

    // Create directory if it doesn't exist
    if !config_dir.exists() {
        fs::create_dir_all(&config_dir)?;
    }

    // Read or create config file
    if config_path.exists() {
        let config_str = fs::read_to_string(&config_path)?;
        Ok(serde_json::from_str(&config_str).unwrap_or_default())
    } else {
        let default_config = FaastaConfig::default();
        let config_str = serde_json::to_string_pretty(&default_config)?;
        fs::write(&config_path, config_str)?;
        Ok(default_config)
    }
}

/// Save the configuration
fn save_config(config: &FaastaConfig) -> Result<(), Error> {
    let config_dir = get_config_dir();
    let config_path = config_dir.join(CONFIG_FILE);

    // Create directory if it doesn't exist
    if !config_dir.exists() {
        fs::create_dir_all(&config_dir)?;
    }

    // Write config
    let config_str = serde_json::to_string_pretty(config)?;
    fs::write(&config_path, config_str)?;

    Ok(())
}

use clap::{Args, Parser, Subcommand};

/// Main entry point
#[tokio::main]
async fn main() {
    let Faasta::Faasta(cli) = Faasta::parse();

    match cli.command {
        Commands::Deploy(args) => {
            let spinner = indicatif::ProgressBar::new_spinner();
            spinner.set_message("Linting project...");
            spinner.enable_steady_tick(std::time::Duration::from_millis(100));

            // Removed lint_project call (analyze crate no longer used)

            spinner.set_message("Deploying project...");

            // Load GitHub config for authentication
            let _github_config = if args.skip_auth {
                None
            } else {
                match load_config() {
                    Ok(config) => {
                        match (config.github_username, config.github_token) {
                            (Some(username), Some(token)) => Some((username, token)),
                            _ => {
                                spinner.finish_and_clear();
                                println!("No GitHub credentials found. Run 'cargo faasta login' to set up authentication.");
                                // println!("Or use --skip-auth to deploy without authentication (limited to one function).");
                                exit(1);
                            }
                        }
                    }
                    Err(e) => {
                        spinner.finish_and_clear();
                        eprintln!("Failed to load config: {}", e);
                        exit(1);
                    }
                }
            };

            // Get package info using cargo metadata
            let output = Command::new("cargo")
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
            let metadata: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
                spinner.finish_and_clear();
                eprintln!("Failed to parse cargo metadata: {}", e);
                exit(1);
            });

            // Extract target_directory
            let target_directory = metadata
                .get("target_directory")
                .and_then(Value::as_str)
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    spinner.finish_and_clear();
                    eprintln!("No 'target_directory' found in cargo metadata");
                    exit(1);
                });

            // Get the package name from the current directory's Cargo.toml
            let packages = metadata
                .get("packages")
                .and_then(Value::as_array)
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
                    let pkg_dir = Path::new(manifest_path).parent()?;
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

            // Path to the WASM file
            // Convert hyphens to underscores in package name for the WASM file
            let wasm_filename = format!("{}.wasm", package_name.replace('-', "_"));
            let wasm_path = target_directory
                .join("wasm32-wasip2")
                .join("release")
                .join(wasm_filename);

            if !wasm_path.exists() {
                spinner.finish_and_clear();
                eprintln!(
                    "Error: Could not find compiled WASM at: {}",
                    wasm_path.display()
                );
                eprintln!("Run 'cargo faasta build' first with wasm32-wasip2 target.");
                exit(1);
            }

            // Read the WASM file
            let wasm_data = match std::fs::read(&wasm_path) {
                Ok(data) => data,
                Err(e) => {
                    spinner.finish_and_clear();
                    eprintln!("Failed to read WASM file: {}", e);
                    exit(1);
                }
            };

            // Get GitHub credentials
            let (github_username, github_token) = if let Some((username, token)) = _github_config {
                (username, token)
            } else {
                spinner.finish_and_clear();
                eprintln!("GitHub credentials required for function upload.");
                exit(1);
            };

            spinner.set_message(format!(
                "Uploading function '{}' to server...",
                package_name
            ));

            // Connect to the function service
            let server_addr = &args.server;

            // Use the connect function to get a client
            let client = match run::connect_to_function_service(server_addr).await {
                Ok(client) => client,
                Err(e) => {
                    spinner.finish_and_clear();
                    eprintln!("Failed to connect to server: {}", e);
                    exit(1);
                }
            };

            // Publish the function
            let auth_token = format!("{}:{}", github_username, github_token);
            match client
                .publish(
                    tarpc::context::current(),
                    wasm_data,
                    package_name.clone(),
                    auth_token,
                )
                .await
            {
                Ok(Ok(message)) => {
                    spinner.finish_and_clear();
                    println!("✅ {}", message);
                    println!("Function URL: {}", format_function_url(&package_name));
                }
                Ok(Err(e)) => {
                    spinner.finish_and_clear();
                    eprintln!("Server error: {:?}", e);
                    exit(1);
                }
                Err(e) => {
                    spinner.finish_and_clear();
                    eprintln!("Communication error: {}", e);
                    exit(1);
                }
            };
        }

        Commands::Invoke(args) => {
            invoke_function(&args.name, &args.arg)
                .await
                .unwrap_or_else(|e| {
                    eprintln!("Failed to invoke function: {}", e);
                    exit(1);
                });
        }

        Commands::Init => {
            let _package_name = "".to_string();

            // Create NewArgs with the current directory's name
            let new_args = NewArgs {
                package_name: _package_name,
            };

            // Delegate to handle_new function
            if let Err(err) = init::handle_new(&new_args) {
                eprintln!("Failed to initialize project in current directory: {err}");
                exit(1);
            }
        }

        Commands::New(new_args) => {
            if let Err(err) = init::handle_new(&new_args) {
                eprintln!("Failed to create new project: {err}");
                exit(1);
            }
        }

        Commands::Build(build_args) => {
            let spinner = indicatif::ProgressBar::new_spinner();
            spinner.set_message("Building project...");
            spinner.enable_steady_tick(std::time::Duration::from_millis(100));

            // Validate the project structure
            if !Path::new("src").join("lib.rs").exists() {
                spinner.finish_and_clear();
                eprintln!(
                    "Error: src/lib.rs is missing. This file is required for FaaSta functions."
                );
                eprintln!("Hint: Run 'cargo faasta new <name>' to create a new FaaSta project.");
                exit(1);
            }

            // Build the project for wasm32-wasip2 target
            spinner.set_message("Building optimized WASI component...");

            // Just use standard cargo build with wasm32-wasip2 target
            let status = Command::new("cargo")
                .args(["build", "--release", "--target", "wasm32-wasip2"])
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

            // If deploy flag is specified, deploy the function
            if build_args.deploy {
                spinner.set_message("Deploying function to server...");

                // Load GitHub config for authentication
                let _github_config = match load_config() {
                    Ok(config) => {
                        match (config.github_username, config.github_token) {
                            (Some(username), Some(token)) => Some((username, token)),
                            _ => {
                                spinner.finish_and_clear();
                                println!("No GitHub credentials found. Run 'cargo faasta login' to set up authentication.");
                                // println!("Or use 'cargo faasta deploy --skip-auth' to deploy without authentication (limited to one function).");
                                None
                            }
                        }
                    }
                    Err(e) => {
                        spinner.finish_and_clear();
                        eprintln!("Failed to load config: {}", e);
                        None
                    }
                };

                // Get package info using cargo metadata
                let output = Command::new("cargo")
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
                let metadata: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
                    spinner.finish_and_clear();
                    eprintln!("Failed to parse cargo metadata: {}", e);
                    exit(1);
                });

                // Extract target_directory
                let target_directory = metadata
                    .get("target_directory")
                    .and_then(Value::as_str)
                    .map(PathBuf::from)
                    .unwrap_or_else(|| {
                        spinner.finish_and_clear();
                        eprintln!("No 'target_directory' found in cargo metadata");
                        exit(1);
                    });

                // Get the package name from the current directory's Cargo.toml
                let packages = metadata
                    .get("packages")
                    .and_then(Value::as_array)
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
                        let pkg_dir = Path::new(manifest_path).parent()?;
                        if same_file_path(
                            &pkg_dir.to_string_lossy(),
                            &current_dir.to_string_lossy(),
                        ) {
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

                // Path to the WASM file
                // Convert hyphens to underscores in package name for the WASM file
                let wasm_filename = format!("{}.wasm", package_name.replace('-', "_"));
                let wasm_path = target_directory
                    .join("wasm32-wasip2")
                    .join("release")
                    .join(wasm_filename);

                if !wasm_path.exists() {
                    spinner.finish_and_clear();
                    eprintln!(
                        "Error: Could not find compiled WASM at: {}",
                        wasm_path.display()
                    );
                    eprintln!("Run 'cargo faasta build' first with wasm32-wasip2 target.");
                    exit(1);
                }

                // Read the WASM file
                let wasm_data = match std::fs::read(&wasm_path) {
                    Ok(data) => data,
                    Err(e) => {
                        spinner.finish_and_clear();
                        eprintln!("Failed to read WASM file: {}", e);
                        exit(1);
                    }
                };

                // Get GitHub credentials
                let (github_username, github_token) =
                    if let Some((username, token)) = _github_config {
                        (username, token)
                    } else {
                        spinner.finish_and_clear();
                        eprintln!("GitHub credentials required for function upload.");
                        exit(1);
                    };

                spinner.set_message(format!(
                    "Uploading function '{}' to server...",
                    package_name
                ));

                // Connect to the function service
                let server_addr = &build_args.server;

                // Use the connect function to get a client
                let client = match run::connect_to_function_service(server_addr).await {
                    Ok(client) => client,
                    Err(e) => {
                        spinner.finish_and_clear();
                        eprintln!("Failed to connect to server: {}", e);
                        exit(1);
                    }
                };

                // Publish the function
                let auth_token = format!("{}:{}", github_username, github_token);
                match client
                    .publish(
                        tarpc::context::current(),
                        wasm_data,
                        package_name.clone(),
                        auth_token,
                    )
                    .await
                {
                    Ok(Ok(message)) => {
                        spinner.finish_and_clear();
                        println!("✅ {}", message);
                        println!("Function URL: {}", format_function_url(&package_name));
                    }
                    Ok(Err(e)) => {
                        spinner.finish_and_clear();
                        eprintln!("Server error: {:?}", e);
                        exit(1);
                    }
                    Err(e) => {
                        spinner.finish_and_clear();
                        eprintln!("Communication error: {}", e);
                        exit(1);
                    }
                };
            } else {
                spinner.finish_and_clear();
                println!("✅ Build successful!");
                println!("Run 'cargo faasta deploy' to deploy your function.");
            }
        }

        Commands::Login(login_args) => {
            // Load existing config or create a new one
            let mut config = match load_config() {
                Ok(cfg) => cfg,
                Err(e) => {
                    eprintln!("Failed to load config: {}", e);
                    exit(1);
                }
            };

            if login_args.manual {
                // Manual login mode - for users who prefer direct token input
                // Set GitHub username
                if let Some(username) = login_args.username {
                    config.github_username = Some(username);
                } else if config.github_username.is_none() {
                    eprintln!("GitHub username required. Use --username to provide it.");
                    exit(1);
                }

                // Set GitHub token
                if let Some(token) = login_args.token {
                    config.github_token = Some(token);
                } else if config.github_token.is_none() {
                    eprintln!("GitHub token required. Use --token to provide it.");
                    exit(1);
                }

                // Save the config
                match save_config(&config) {
                    Ok(_) => {
                        println!("GitHub credentials saved successfully.");
                        println!(
                            "You can now deploy up to {} projects.",
                            MAX_PROJECTS_PER_USER
                        );
                    }
                    Err(e) => {
                        eprintln!("Failed to save config: {}", e);
                        exit(1);
                    }
                }
            } else {
                // Interactive OAuth flow
                match crate::github_oauth::github_oauth_flow().await {
                    Ok((username, token)) => {
                        config.github_username = Some(username);
                        config.github_token = Some(token);

                        match save_config(&config) {
                            Ok(_) => {
                                println!("✅ GitHub authentication successful!");
                                println!(
                                    "You can now deploy up to {} projects.",
                                    MAX_PROJECTS_PER_USER
                                );
                            }
                            Err(e) => {
                                eprintln!("Failed to save config: {}", e);
                                exit(1);
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("GitHub authentication failed: {}", e);
                        eprintln!("Try again or use manual login: cargo faasta login --manual --username <user> --token <token>");
                        exit(1);
                    }
                }
            }
        }

        Commands::Run(run_args) => {
            // Call the run module handler
            run::handle_run(run_args.port).await.unwrap_or_else(|e| {
                eprintln!("Failed to run function: {}", e);
                exit(1);
            });
        }
    }
}

#[derive(Args, Debug)]
pub struct NewArgs {
    /// The name of the package to create
    package_name: String,
}

#[derive(Args, Debug)]
pub struct LoginArgs {
    /// GitHub username (only needed for manual login)
    #[arg(long)]
    username: Option<String>,

    /// GitHub token (only needed for manual login)
    #[arg(long)]
    token: Option<String>,

    /// Skip browser OAuth flow and manually provide credentials
    #[arg(long)]
    manual: bool,
}

#[derive(Parser)] // requires `derive` feature
#[command(name = "cargo")]
#[command(bin_name = "cargo")]
#[command(styles = CLAP_STYLING)]
enum Faasta {
    #[command(name = "faasta")]
    Faasta(Cli),
}

#[derive(Args, Debug)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Deploys a project to the server
    Deploy(DeployArgs),
    /// Invokes a function with the specified name and argument
    Invoke(InvokeArgs),
    /// Initialize a new project in the current directory
    Init,
    /// Create a new project in a new directory
    New(NewArgs),
    /// Build the function (and optionally deploy it)
    Build(BuildArgs),
    /// Set up GitHub authentication
    Login(LoginArgs),
    /// Run a function locally for testing
    Run(RunArgs),
}

#[derive(Args, Debug)]
struct DeployArgs {
    /// Path to the project to deploy
    path: Option<String>,

    /// Skip GitHub authentication
    #[arg(long)]
    skip_auth: bool,

    /// Server address to deploy to (e.g., "faasta.xyz:4433")
    #[arg(long, default_value = "faasta.xyz:4433")]
    server: String,
}

#[derive(Args, Debug)]
struct BuildArgs {
    /// Deploy the function after building
    #[arg(short, long)]
    deploy: bool,

    /// Server address to deploy to (e.g., "faasta.xyz:4433")
    #[arg(long, default_value = "faasta.xyz:4433")]
    server: String,
}

#[derive(Args, Debug)]
struct RunArgs {
    /// Port to run the local server on
    #[arg(short, long, default_value = "3000")]
    port: u16,
}

#[derive(Args, Debug)]
struct InvokeArgs {
    /// Name of the function to invoke
    name: String,
    /// Optional argument to pass to the function
    #[arg(default_value = "")]
    arg: String,
}

/// Custom styling for the CLI
pub const CLAP_STYLING: clap::builder::styling::Styles = clap::builder::styling::Styles::styled()
    .header(clap_cargo::style::HEADER)
    .usage(clap_cargo::style::USAGE)
    .literal(clap_cargo::style::LITERAL)
    .placeholder(clap_cargo::style::PLACEHOLDER)
    .error(clap_cargo::style::ERROR)
    .valid(clap_cargo::style::VALID)
    .invalid(clap_cargo::style::INVALID);

/// Formats the function URL based on the INVOKE_URL
/// If INVOKE_URL is a domain (not localhost or an IP), it uses function_name as a subdomain
/// Otherwise, it appends function_name as a path
fn format_function_url(function_name: &str) -> String {
    // Parse the INVOKE_URL to get the hostname
    // Format: scheme://host/path
    let url_parts: Vec<&str> = INVOKE_URL.split("://").collect();
    if url_parts.len() != 2 {
        // If URL doesn't follow the expected format, fall back to the original behavior
        return format!("{}{}", INVOKE_URL, function_name);
    }

    let scheme = url_parts[0];
    let rest = url_parts[1];

    // Split host and path
    let host_path_parts: Vec<&str> = rest.split('/').collect();
    let host = host_path_parts[0];

    // Check if host is localhost or an IP address
    if host == "localhost" || host == "127.0.0.1" || is_ip_address(host) {
        // For localhost or IP, append function_name as a path
        let base = if INVOKE_URL.ends_with('/') {
            INVOKE_URL.to_string()
        } else {
            format!("{}/", INVOKE_URL)
        };
        format!("{}{}", base, function_name)
    } else {
        // For a domain name, use function_name as a subdomain
        format!("{}://{}.{}/", scheme, function_name, host)
    }
}

/// Check if a host string is an IP address
fn is_ip_address(host: &str) -> bool {
    host.parse::<std::net::IpAddr>().is_ok()
}

async fn invoke_function(name: &str, arg: &str) -> Result<(), reqwest::Error> {
    let function_url = format_function_url(name);
    let invoke_url = if function_url.ends_with('/') {
        format!("{}{}", function_url, arg)
    } else {
        format!("{}/{}", function_url, arg)
    };

    println!("Invoking function at: {}", invoke_url);

    // Create a client that accepts invalid certificates (for testing)
    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .build()?;

    // Make sure we're using HTTPS
    let https_url = if !invoke_url.starts_with("https://") && !invoke_url.starts_with("http://") {
        format!("https://{}", invoke_url)
    } else if invoke_url.starts_with("http://") {
        invoke_url.replace("http://", "https://")
    } else {
        invoke_url
    };

    let resp = client.get(https_url).send().await?;
    println!("Response status: {}", resp.status());
    println!("{}", resp.text().await?);
    Ok(())
}

/// Find a workspace root package if it exists; otherwise pick the current/only package from cargo metadata.
///
/// Compare two file paths in a slightly more robust way.
/// (On Windows, e.g., backslash vs forward slash).
fn same_file_path(a: &str, b: &str) -> bool {
    // Convert both to a canonical PathBuf
    let path_a = Path::new(a).components().collect::<Vec<_>>();
    let path_b = Path::new(b).components().collect::<Vec<_>>();
    path_a == path_b
}
