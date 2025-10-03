#![warn(unused_extern_crates)]
mod github_oauth;
mod init;
mod run;

use anyhow::Error;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::process::exit;

const DEFAULT_INVOKE_URL: &str = "https://faasta.xyz/";
const MAX_PROJECTS_PER_USER: usize = 10;
const CONFIG_DIR: &str = ".faasta";
const CONFIG_FILE: &str = "config.json";

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

use crate::init::NewArgs;
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
                        eprintln!("Failed to load config: {e}");
                        exit(1);
                    }
                }
            };

            // Get project information
            let (target_directory, package_name, _) = match run::get_project_info() {
                Ok(info) => info,
                Err(e) => {
                    spinner.finish_and_clear();
                    eprintln!("Failed to get project information: {e}");
                    exit(1);
                }
            };

            // Path to the WASM file
            // Note: Rust compiler output converts hyphens to underscores, so we need to
            // handle this conversion to find the compiled WASM file
            // This is a client-side only conversion that's needed to locate the compiled artifact
            let wasm_path = if let Some(explicit_path) = &args.wasm_path {
                // User provided an explicit WASM path
                PathBuf::from(explicit_path)
            } else {
                // Auto-detect based on package name
                let rust_compiled_name = package_name.replace('-', "_");
                let wasm_filename = format!("{rust_compiled_name}.wasm");

                // Path to the compiled WASM file (uses Rust's converted name)
                target_directory
                    .join("wasm32-wasip2")
                    .join("release")
                    .join(wasm_filename)
            };

            // For explicit WASM paths, we'll use the filename without extension as the function name
            // unless the user specified a function name
            let function_name = if args.wasm_path.is_some() && args.function_name.is_some() {
                // User provided both WASM path and function name - use the explicit function name
                args.function_name.clone().unwrap()
            } else if args.wasm_path.is_some() {
                // User provided WASM path but no function name - derive from filename
                wasm_path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .map(|s| s.to_owned())
                    .unwrap_or_else(|| {
                        spinner.finish_and_clear();
                        eprintln!("Error: Could not determine function name from WASM filename");
                        exit(1);
                    })
            } else {
                // Standard flow - use the package name
                package_name.clone()
            };

            spinner.set_message(format!(
            "Uploading function '{function_name}' to server..."
        ));

            if !wasm_path.exists() {
                spinner.finish_and_clear();
                if args.wasm_path.is_some() {
                    eprintln!(
                        "Error: Could not find WASM file at: {}",
                        wasm_path.display()
                    );
                } else {
                    eprintln!(
                        "Error: Could not find compiled WASM at: {}",
                        wasm_path.display()
                    );
                    eprintln!("Options:");
                    eprintln!("  1. Run 'cargo faasta build' first with wasm32-wasip2 target");
                    eprintln!("  2. Specify an explicit WASM file path with --wasm-path");
                    eprintln!();
                    eprintln!("If your WASM file is in a non-standard location or has a different name, use:");
                    eprintln!("  cargo faasta deploy --wasm-path PATH/TO/YOUR/FILE.wasm");
                }
                exit(1);
            }

            // Read the WASM file
            let wasm_data = match std::fs::read(&wasm_path) {
                Ok(data) => {
                    // Check WASM file size client-side as well (30MB max)
                    if data.len() > faasta_interface::MAX_WASM_SIZE {
                        spinner.finish_and_clear();
                        eprintln!(
                            "Error: WASM file too large ({}MB). Maximum allowed size is 30MB.",
                            data.len() / 1024 / 1024
                        );
                        exit(1);
                    }
                    data
                }
                Err(e) => {
                    spinner.finish_and_clear();
                    eprintln!("Failed to read WASM file: {e}");
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

            // Connect to the function service
            let server_addr = &args.server;

            // Use the connect function to get a client
            let client = match run::connect_to_function_service(server_addr).await {
                Ok(client) => client,
                Err(e) => {
                    spinner.finish_and_clear();
                    eprintln!("Failed to connect to server: {e}");
                    exit(1);
                }
            };

            // Publish the function
            let auth_token = format!("{github_username}:{github_token}");
            match client
                .publish(wasm_data, function_name.clone(), auth_token)
                .await
            {
                Ok(Ok(message)) => {
                    spinner.finish_and_clear();
                    println!("✅ {message}");

                    // Extract server hostname from server address (remove port)
                    let server_host = extract_server_host(&args.server);
                    println!(
                        "Function URL: {}",
                        format_function_url(&function_name, &server_host)
                    );
                }
                Ok(Err(e)) => {
                    spinner.finish_and_clear();
                    eprintln!("Server error: {e:?}");
                    exit(1);
                }
                Err(e) => {
                    spinner.finish_and_clear();
                    eprintln!("Communication error: {e}");
                    exit(1);
                }
            };
        }

        Commands::Invoke(args) => {
            invoke_function(&args.name, &args.arg)
                .await
                .unwrap_or_else(|e| {
                    eprintln!("Failed to invoke function: {e}");
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

            // Get project information
            let (target_directory, package_name, package_root) = match run::get_project_info() {
                Ok(info) => info,
                Err(e) => {
                    spinner.finish_and_clear();
                    eprintln!("Failed to get project information: {e}");
                    exit(1);
                }
            };

            // Build the project
            if let Err(e) = run::build_project(&package_root) {
                spinner.finish_and_clear();
                eprintln!("Failed to build project: {e}");
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
                        eprintln!("Failed to load config: {e}");
                        None
                    }
                };

                // Path to the WASM file
                // Note: Rust compiler output converts hyphens to underscores, so we need to
                // handle this conversion to find the compiled WASM file
                let wasm_path = if let Some(explicit_path) = &build_args.wasm_path {
                    // User provided an explicit WASM path
                    PathBuf::from(explicit_path)
                } else {
                    // Auto-detect based on package name
                    let rust_compiled_name = package_name.replace('-', "_");
                    let wasm_filename = format!("{rust_compiled_name}.wasm");

                    // Path to the compiled WASM file (uses Rust's converted name)
                    target_directory
                        .join("wasm32-wasip2")
                        .join("release")
                        .join(wasm_filename)
                };

                // For explicit WASM paths, we'll use the filename without extension as the function name
                // unless the user specified a function name
                let function_name =
                    if build_args.wasm_path.is_some() && build_args.function_name.is_some() {
                        // User provided both WASM path and function name - use the explicit function name
                        build_args.function_name.clone().unwrap()
                    } else if build_args.wasm_path.is_some() {
                        // User provided WASM path but no function name - derive from filename
                        wasm_path
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .map(|s| s.to_owned())
                            .unwrap_or_else(|| {
                                spinner.finish_and_clear();
                                eprintln!(
                                    "Error: Could not determine function name from WASM filename"
                                );
                                exit(1);
                            })
                    } else {
                        // Standard flow - use the package name
                        package_name.clone()
                    };

                if !wasm_path.exists() {
                    spinner.finish_and_clear();
                    if build_args.wasm_path.is_some() {
                        eprintln!(
                            "Error: Could not find WASM file at: {}",
                            wasm_path.display()
                        );
                    } else {
                        eprintln!(
                            "Error: Could not find compiled WASM at: {}",
                            wasm_path.display()
                        );
                        eprintln!("Options:");
                        eprintln!("  1. Run 'cargo faasta build' first with wasm32-wasip2 target");
                        eprintln!("  2. Specify an explicit WASM file path with --wasm-path");
                        eprintln!();
                        eprintln!("If your WASM file is in a non-standard location or has a different name, use:");
                        eprintln!(
                            "  cargo faasta build --deploy --wasm-path PATH/TO/YOUR/FILE.wasm"
                        );
                    }
                    exit(1);
                }

                // Read the WASM file
                let wasm_data = match std::fs::read(&wasm_path) {
                    Ok(data) => {
                        // Check WASM file size client-side as well (30MB max)
                        if data.len() > faasta_interface::MAX_WASM_SIZE {
                            spinner.finish_and_clear();
                            eprintln!(
                                "Error: WASM file too large ({}MB). Maximum allowed size is 30MB.",
                                data.len() / 1024 / 1024
                            );
                            exit(1);
                        }
                        data
                    }
                    Err(e) => {
                        spinner.finish_and_clear();
                        eprintln!("Failed to read WASM file: {e}");
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
                    "Uploading function '{function_name}' to server..."
                ));

                // Connect to the function service
                let server_addr = &build_args.server;

                // Use the connect function to get a client
                let client = match run::connect_to_function_service(server_addr).await {
                    Ok(client) => client,
                    Err(e) => {
                        spinner.finish_and_clear();
                        eprintln!("Failed to connect to server: {e}");
                        exit(1);
                    }
                };

                // Publish the function
                let auth_token = format!("{github_username}:{github_token}");
                match client
                    .publish(wasm_data, function_name.clone(), auth_token)
                    .await
                {
                    Ok(Ok(message)) => {
                        spinner.finish_and_clear();
                        println!("✅ {message}");

                        // Extract server hostname from server address (remove port)
                        let server_host = extract_server_host(&build_args.server);
                        println!(
                            "Function URL: {}",
                            format_function_url(&function_name, &server_host)
                        );
                    }
                    Ok(Err(e)) => {
                        spinner.finish_and_clear();
                        eprintln!("Server error: {e:?}");
                        exit(1);
                    }
                    Err(e) => {
                        spinner.finish_and_clear();
                        eprintln!("Communication error: {e}");
                        exit(1);
                    }
                };
            }
        }

        Commands::Login(login_args) => {
            // Load existing config or create a new one
            let mut config = match load_config() {
                Ok(cfg) => cfg,
                Err(e) => {
                    eprintln!("Failed to load config: {e}");
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
                            "You can now deploy up to {MAX_PROJECTS_PER_USER} projects."
                        );
                    }
                    Err(e) => {
                        eprintln!("Failed to save config: {e}");
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
                                    "You can now deploy up to {MAX_PROJECTS_PER_USER} projects."
                                );
                            }
                            Err(e) => {
                                eprintln!("Failed to save config: {e}");
                                exit(1);
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("GitHub authentication failed: {e}");
                        eprintln!("Try again or use manual login: cargo faasta login --manual --username <user> --token <token>");
                        exit(1);
                    }
                }
            }
        }

        Commands::Metrics(args) => {
            let spinner = indicatif::ProgressBar::new_spinner();
            spinner.set_message("Fetching metrics...");
            spinner.enable_steady_tick(std::time::Duration::from_millis(100));

            // Load GitHub config for authentication
            let github_config = match load_config() {
                Ok(config) => match (config.github_username, config.github_token) {
                    (Some(username), Some(token)) => Some((username, token)),
                    _ => {
                        spinner.finish_and_clear();
                        println!("No GitHub credentials found. Run 'cargo faasta login' to set up authentication.");
                        exit(1);
                    }
                },
                Err(e) => {
                    spinner.finish_and_clear();
                    eprintln!("Failed to load config: {e}");
                    exit(1);
                }
            };

            // Get GitHub credentials
            let (github_username, github_token) = github_config.unwrap();

            // Connect to the server
            let client = match run::connect_to_function_service(&args.server).await {
                Ok(client) => client,
                Err(e) => {
                    spinner.finish_and_clear();
                    eprintln!("Failed to connect to server: {e}");
                    exit(1);
                }
            };

            // Call get_metrics
            spinner.finish_and_clear();
            if let Err(e) = get_metrics(&client, &github_username, &github_token).await {
                eprintln!("Error fetching metrics: {e}");
                exit(1);
            }
        }

        Commands::Unpublish(args) => {
            let spinner = indicatif::ProgressBar::new_spinner();
            spinner.set_message(format!("Unpublishing function '{}'...", args.name));
            spinner.enable_steady_tick(std::time::Duration::from_millis(100));

            // Load GitHub config for authentication
            let github_config = match load_config() {
                Ok(config) => match (config.github_username, config.github_token) {
                    (Some(username), Some(token)) => Some((username, token)),
                    _ => {
                        spinner.finish_and_clear();
                        println!("No GitHub credentials found. Run 'cargo faasta login' to set up authentication.");
                        exit(1);
                    }
                },
                Err(e) => {
                    spinner.finish_and_clear();
                    eprintln!("Failed to load config: {e}");
                    exit(1);
                }
            };

            // Get GitHub credentials
            let (github_username, github_token) = github_config.unwrap();

            // Connect to the function service
            let client = match run::connect_to_function_service(&args.server).await {
                Ok(client) => client,
                Err(e) => {
                    spinner.finish_and_clear();
                    eprintln!("Failed to connect to server: {e}");
                    exit(1);
                }
            };

            // Create auth token (username:token format)
            let auth_token = format!("{github_username}:{github_token}");

            // Call the unpublish RPC
            match client
                .unpublish(args.name.clone(), auth_token)
                .await
            {
                Ok(Ok(_)) => {
                    spinner.finish_and_clear();
                    println!("✅ Function '{}' unpublished successfully", args.name);
                }
                Ok(Err(e)) => {
                    spinner.finish_and_clear();
                    match e {
                        faasta_interface::FunctionError::NotFound(_) => {
                            eprintln!("Error: Function '{}' not found", args.name)
                        }
                        faasta_interface::FunctionError::PermissionDenied(_) => {
                            eprintln!("Error: You don't have permission to unpublish this function")
                        }
                        _ => eprintln!("Server error: {e:?}"),
                    }
                    exit(1);
                }
                Err(e) => {
                    spinner.finish_and_clear();
                    eprintln!("Communication error: {e}");
                    exit(1);
                }
            }
        }

        Commands::List(args) => {
            let spinner = indicatif::ProgressBar::new_spinner();
            spinner.set_message("Fetching function list...");
            spinner.enable_steady_tick(std::time::Duration::from_millis(100));

            // Load GitHub config for authentication
            let github_config = match load_config() {
                Ok(config) => match (config.github_username, config.github_token) {
                    (Some(username), Some(token)) => Some((username, token)),
                    _ => {
                        spinner.finish_and_clear();
                        println!("No GitHub credentials found. Run 'cargo faasta login' to set up authentication.");
                        exit(1);
                    }
                },
                Err(e) => {
                    spinner.finish_and_clear();
                    eprintln!("Failed to load config: {e}");
                    exit(1);
                }
            };

            // Get GitHub credentials
            let (github_username, github_token) = github_config.unwrap();

            // Connect to the server
            let client = match run::connect_to_function_service(&args.server).await {
                Ok(client) => client,
                Err(e) => {
                    spinner.finish_and_clear();
                    eprintln!("Failed to connect to server: {e}");
                    exit(1);
                }
            };

            // Call list_functions
            spinner.finish_and_clear();
            if let Err(e) = list_functions(&client, &github_username, &github_token).await {
                eprintln!("Error listing functions: {e}");
                exit(1);
            }
        }

        Commands::Run(run_args) => {
            // Call the run module handler
            run::handle_run(run_args.port).await.unwrap_or_else(|e| {
                eprintln!("Failed to run function: {e}");
                exit(1);
            });
        }
    }
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
    /// Get metrics for deployed functions
    Metrics(ServerArgs),
    /// List all functions deployed under the current GitHub account
    List(ServerArgs),
    /// Run a function locally for testing
    Run(RunArgs),
    /// Unpublish a function from the server
    Unpublish(UnpublishArgs),
}

#[derive(Args, Debug)]
struct DeployArgs {
    /// Path to the project to deploy
    path: Option<String>,

    /// Skip GitHub authentication
    #[arg(long)]
    skip_auth: bool,

    /// Explicit path to WASM file (overrides automatic detection)
    #[arg(long)]
    wasm_path: Option<String>,

    /// Function name to use (if different from package name)
    #[arg(long)]
    function_name: Option<String>,

    /// Server address to deploy to (e.g., "faasta.xyz:4433")
    #[arg(long, default_value = "faasta.xyz:4433")]
    server: String,
}

#[derive(Args, Debug)]
struct BuildArgs {
    /// Deploy the function after building
    #[arg(short, long)]
    deploy: bool,

    /// Explicit path to WASM file (overrides automatic detection)
    #[arg(long)]
    wasm_path: Option<String>,

    /// Function name to use (if different from package name)
    #[arg(long)]
    function_name: Option<String>,

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

#[derive(Args, Debug)]
struct UnpublishArgs {
    /// Name of the function to unpublish
    name: String,
    /// Server address (e.g., "faasta.xyz:4433")
    #[arg(long, default_value = "faasta.xyz:4433")]
    server: String,
}

#[derive(Args, Debug)]
struct ServerArgs {
    /// Server address (e.g., "faasta.xyz:4433")
    #[arg(long, default_value = "faasta.xyz:4433")]
    server: String,
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

/// Formats the function URL based on the server URL
/// If server is a domain (not localhost or an IP), it uses function_name as a subdomain
/// Otherwise, it appends function_name as a path
fn format_function_url(function_name: &str, server: &str) -> String {
    // Ensure server has a scheme
    let server_url = if !server.contains("://") {
        format!("https://{server}")
    } else {
        server.to_string()
    };

    // Parse the server URL to get the hostname
    // Format: scheme://host/path
    let url_parts: Vec<&str> = server_url.split("://").collect();
    if url_parts.len() != 2 {
        // If URL doesn't follow the expected format, fall back to a simple approach
        return format!("https://{server}/{function_name}");
    }

    let scheme = url_parts[0];
    let rest = url_parts[1];

    // Split host and path
    let host_path_parts: Vec<&str> = rest.split('/').collect();
    let host = host_path_parts[0];

    // Check if host is localhost or an IP address
    if host == "localhost" || host == "127.0.0.1" || is_ip_address(host) {
        // For localhost or IP, append function_name as a path
        let base = if server_url.ends_with('/') {
            server_url
        } else {
            format!("{server_url}/")
        };
        format!("{base}{function_name}")
    } else {
        // For a domain name, use function_name as a subdomain
        format!("{scheme}://{function_name}.{host}/")
    }
}

/// Extract the server host from a server address (removing any port)
fn extract_server_host(server_addr: &str) -> String {
    // If it already has a scheme, use it as is
    if server_addr.contains("://") {
        return server_addr.to_string();
    }

    // Remove port if present
    if let Some(host) = server_addr.split(':').next() {
        format!("https://{host}")
    } else {
        format!("https://{server_addr}")
    }
}

/// Check if a host string is an IP address
fn is_ip_address(host: &str) -> bool {
    host.parse::<std::net::IpAddr>().is_ok()
}

async fn invoke_function(name: &str, arg: &str) -> Result<(), reqwest::Error> {
    let function_url = format_function_url(name, DEFAULT_INVOKE_URL);
    let invoke_url = if function_url.ends_with('/') {
        format!("{function_url}{arg}")
    } else {
        format!("{function_url}/{arg}")
    };

    println!("Invoking function at: {invoke_url}");

    // Create a client that accepts invalid certificates (for testing)
    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .build()?;

    // Make sure we're using HTTPS
    let https_url = if !invoke_url.starts_with("https://") && !invoke_url.starts_with("http://") {
        format!("https://{invoke_url}")
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

// Function to fetch and display metrics
async fn get_metrics(
    client: &run::FunctionServiceClient,
    username: &str,
    token: &str,
) -> anyhow::Result<()> {
    // Create auth token (username:token format)
    let auth_token = format!("{username}:{token}");

    println!("Fetching metrics from server...");

    // Call the get_metrics RPC
    match client.get_metrics(auth_token).await {
        Ok(Ok(metrics)) => {
            // Print summary
            println!("\n╔══════════════════════════════════════════════════════");
            println!("║ FAASTA FUNCTION METRICS");
            println!("╠══════════════════════════════════════════════════════");
            println!("║ Total Function Calls: {}", metrics.total_calls);

            // Format total execution time nicely
            let total_time = if metrics.total_time > 60000 {
                format!("{:.2} minutes", metrics.total_time as f64 / 60000.0)
            } else if metrics.total_time > 1000 {
                format!("{:.2} seconds", metrics.total_time as f64 / 1000.0)
            } else {
                format!("{} ms", metrics.total_time)
            };

            println!("║ Total Execution Time: {total_time}");
            println!("║ Functions Deployed: {}", metrics.function_metrics.len());
            println!("╠══════════════════════════════════════════════════════");

            // If we have no functions, show a message
            if metrics.function_metrics.is_empty() {
                println!("║ No function metrics available.");
                println!("╚══════════════════════════════════════════════════════");
                return Ok(());
            }

            // Print detailed metrics for each function
            println!("║ FUNCTION DETAILS");
            println!("╠══════════════════════════════════════════════════════");

            for function in metrics.function_metrics {
                println!("║ Function: {}", function.function_name);
                println!("║ ├─ Call Count: {}", function.call_count);

                // Format execution time nicely
                let exec_time = if function.total_time_millis > 60000 {
                    format!("{:.2} minutes", function.total_time_millis as f64 / 60000.0)
                } else if function.total_time_millis > 1000 {
                    format!("{:.2} seconds", function.total_time_millis as f64 / 1000.0)
                } else {
                    format!("{} ms", function.total_time_millis)
                };

                println!("║ ├─ Total Execution Time: {exec_time}");

                // Format average time per call
                let avg_time = if function.call_count > 0 {
                    format!(
                        "{:.2} ms",
                        function.total_time_millis as f64 / function.call_count as f64
                    )
                } else {
                    "N/A".to_string()
                };

                println!("║ ├─ Average Time per Call: {avg_time}");
                println!("║ └─ Last Called: {}", function.last_called);
                println!("╟──────────────────────────────────────────────────────");
            }
            println!("╚══════════════════════════════════════════════════════");
            Ok(())
        }
        Ok(Err(e)) => {
            eprintln!("Server error: {e:?}");
            Err(anyhow::anyhow!("Server error: {:?}", e))
        }
        Err(e) => Err(anyhow::anyhow!("Communication error: {}", e)),
    }
}

// Function to fetch and display list of functions
async fn list_functions(
    client: &run::FunctionServiceClient,
    username: &str,
    token: &str,
) -> anyhow::Result<()> {
    // Create auth token (username:token format)
    let auth_token = format!("{username}:{token}");

    println!("Fetching functions for GitHub user: {username}...");

    // Call the list_functions RPC
    match client.list_functions(auth_token).await {
        Ok(Ok(functions)) => {
            if functions.is_empty() {
                println!("\nNo functions deployed under this GitHub account.");
                println!("Use 'cargo faasta deploy' to deploy a function.");
                return Ok(());
            }

            // Print header
            println!("\n╔══════════════════════════════════════════════════════");
            println!("║ FUNCTIONS DEPLOYED BY {}", username.to_uppercase());
            println!("╠══════════════════════════════════════════════════════");
            println!("║ Total Functions: {}", functions.len());
            println!("╠══════════════════════════════════════════════════════");

            // Print functions in alphabetical order
            let mut sorted_functions = functions.clone();
            sorted_functions.sort_by(|a, b| a.name.cmp(&b.name));

            for function in sorted_functions {
                println!("║ Function: {}", function.name);

                // Parse the published_at date for pretty formatting
                println!("║ ├─ Published: {}", function.published_at);

                // URL
                println!("║ ├─ URL: {}", function.usage);

                // Add a command to invoke it
                println!("║ └─ Invoke: cargo faasta invoke {}", function.name);
                println!("╟──────────────────────────────────────────────────────");
            }
            println!("╚══════════════════════════════════════════════════════");

            Ok(())
        }
        Ok(Err(e)) => Err(anyhow::anyhow!("Server error: {:?}", e)),
        Err(e) => Err(anyhow::anyhow!("Communication error: {}", e)),
    }
}
