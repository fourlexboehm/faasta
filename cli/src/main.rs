mod init;
mod github_oauth;
#[cfg(test)]
mod test_build;

use faasta_analyze::lint_project;
use anyhow::Error;
use reqwest::{header, multipart, Client};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use faasta_analyze::build_project;
use std::process::{exit, Command};
use std::{env, fmt};
use walkdir::WalkDir;
use zip::write::{ExtendedFileOptions, FileOptions};
use zip::{CompressionMethod, ZipWriter};

const UPLOAD_URL: &str = "http://127.0.0.1:8080/upload";
const INVOKE_URL: &str = "http://127.0.0.1:8080/";
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

#[derive(Serialize, Deserialize, Debug, Clone)]
struct FaastaConfig {
    github_username: Option<String>,
    github_token: Option<String>,
}

impl Default for FaastaConfig {
    fn default() -> Self {
        Self {
            github_username: None,
            github_token: None,
        }
    }
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

/// Recursively walk the given `project_path` and upload all files (including `Cargo.toml`, `src/` content, etc.).
/// Each file is added to the multipart form with a field name that is the **relative path** from `project_path`.
/// Zips up the local project (skipping `target/` and build scripts)
/// and uploads it as a single multipart form field named `"archive"`.
pub async fn upload_project(github_config: Option<(String, String)>) -> Result<String, Error> {
    let (package_root, package_name) = find_root_package().unwrap();

    // 1) Create an in-memory buffer that we'll write the zip to.
    let mut buffer = Vec::new();
    {
        let cursor = Cursor::new(&mut buffer);
        let mut zip = ZipWriter::new(cursor);

        // 2) Walk the project directory and add each file to the zip,
        //    except for files under `target/` or named `build.rs`.
        let options: FileOptions<'_, ExtendedFileOptions> = FileOptions::default()
            .compression_method(CompressionMethod::Deflated)
            .unix_permissions(0o644);

        for entry in WalkDir::new(&package_root)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            // Skip directories; only handle files
            if !path.is_file() {
                continue;
            }

            // Skip `target/` directory
            if path.strip_prefix(&package_root)
                .ok()
                .and_then(|rel| rel.to_str())
                .map(|s| s.starts_with("target/"))
                .unwrap_or(false)
            {
                continue;
            }

            if path.file_name().and_then(|n| n.to_str()) == Some("build.rs") {
                println!("Skipping build.rs, build time code is unsupported");
                continue;
            }

            // Derive the relative path from package_root so we can store that
            // exact path in the zip. For example, `src/main.rs`.
            let relative_path = match path.strip_prefix(&package_root) {
                Ok(rp) => rp,
                Err(_) => continue,
            };

            // Add a file entry to the zip
            zip.start_file(
                relative_path.to_string_lossy(),
                options.clone()
            )?;

            // Write the file contents into the zip.
            let bytes = std::fs::read(path)?;
            zip.write_all(&bytes)?;
        }

        // Finalize the ZIP
        zip.finish()?;
    } // Drop `zip` so that `buffer` is complete

    // 3) Build a multipart form with a single part containing our in-memory ZIP.
    let client = Client::new();
    let zip_part = multipart::Part::bytes(buffer)
        // The actual filename on the server is up to you;
        // you can name it e.g. `<crate_name>.zip` or "project.zip"
        .file_name(format!("{}.zip", package_name));

    let form = multipart::Form::new()
        .part("archive", zip_part);

    // 4) Send the POST request with our zip file in the form
    let url = format!("{}/{}", UPLOAD_URL, package_name);
    let mut request = client.post(&url).multipart(form);
    
    // Add GitHub authentication if available
    if let Some((username, token)) = github_config {
        request = request
            .header("X-GitHub-Username", username)
            .header(header::AUTHORIZATION, token);
    }
    
    let response = request.send().await?;

    // 5) Return the response body as text, or handle it however needed
    let text = response.text().await?;
    println!("Server response: {text}");
    println!("Function URL: {}{}", INVOKE_URL, package_name);

    Ok(text)
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

            lint_project(&env::current_dir().unwrap()).await.unwrap_or_else(|e| {
                spinner.finish_and_clear();
                eprintln!("Failed to lint project: {}", e);
                exit(1);
            });

            spinner.set_message("Deploying project...");
            
            // Load GitHub config for authentication
            let github_config = if args.skip_auth {
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
                    },
                    Err(e) => {
                        spinner.finish_and_clear();
                        eprintln!("Failed to load config: {}", e);
                        exit(1);
                    }
                }
            };
            
            upload_project(github_config).await.unwrap_or_else(|e| {
                spinner.finish_and_clear();
                eprintln!("Failed to deploy project: {}", e);
                exit(1);
            });

            let (_, package_name) = find_root_package().expect("Failed to find root package");
            spinner.finish_and_clear();
            println!("✅ Function '{}' deployed successfully", package_name);
            println!("Function URL: {}{}", INVOKE_URL, package_name);
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
        },
        
        Commands::Build(build_args) => {
            let spinner = indicatif::ProgressBar::new_spinner();
            spinner.set_message("Building project...");
            spinner.enable_steady_tick(std::time::Duration::from_millis(100));

            let (package_root, package_name) = find_root_package().expect("Failed to find root package");
            
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

            // If deploy flag is specified, deploy the function
            if build_args.deploy {
                spinner.set_message("Deploying function to server...");
                
                // Load GitHub config for authentication
                let github_config = match load_config() {
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
                    },
                    Err(e) => {
                        spinner.finish_and_clear();
                        eprintln!("Failed to load config: {}", e);
                        None
                    }
                };
                
                match upload_project(github_config).await {
                    Ok(_) => {
                        spinner.finish_and_clear();
                        println!("✅ Function '{}' deployed successfully", package_name);
                        println!("Function URL: {}{}", INVOKE_URL, package_name);
                    },
                    Err(e) => {
                        spinner.finish_and_clear();
                        eprintln!("Deployment failed: {}", e);
                        eprintln!("The build succeeded but deployment failed. You can deploy later with 'cargo faasta deploy'");
                        exit(1);
                    }
                }
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
                        println!("You can now deploy up to {} projects.", MAX_PROJECTS_PER_USER);
                    },
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
                                println!("You can now deploy up to {} projects.", MAX_PROJECTS_PER_USER);
                            },
                            Err(e) => {
                                eprintln!("Failed to save config: {}", e);
                                exit(1);
                            }
                        }
                    },
                    Err(e) => {
                        eprintln!("GitHub authentication failed: {}", e);
                        eprintln!("Try again or use manual login: cargo faasta login --manual --username <user> --token <token>");
                        exit(1);
                    }
                }
            }
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
}

#[derive(Args, Debug)]
struct DeployArgs {
    /// Path to the project to deploy
    path: Option<String>,
    
    /// Skip GitHub authentication
    #[arg(long)]
    skip_auth: bool,
}

#[derive(Args, Debug)]
struct BuildArgs {
    /// Deploy the function after building
    #[arg(short, long)]
    deploy: bool,
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

async fn invoke_function(name: &str, arg: &str) -> Result<(), reqwest::Error> {
    let resp = reqwest::get(&format!("{}{}/{}", INVOKE_URL, name, arg)).await?;
    println!("{:?}", resp.text().await?);
    Ok(())
}

/// Find a workspace root package if it exists; otherwise pick the
/// current/only package from cargo metadata.
fn find_root_package() -> Result<(PathBuf, String), Box<dyn std::error::Error>> {
    // Run `cargo metadata --format-version=1`
    let output = Command::new("cargo")
        .args(["metadata", "--format-version=1"])
        .output()?;

    if !output.status.success() {
        return Err("Failed to retrieve cargo metadata".into());
    }

    // Parse JSON
    let v: Value = serde_json::from_slice(&output.stdout)?;

    // Extract workspace_root
    let Some(workspace_root_str) = v.get("workspace_root").and_then(Value::as_str) else {
        return Err("No 'workspace_root' found in cargo metadata".into());
    };
    let workspace_root = PathBuf::from(workspace_root_str);

    // Look through the "packages" array
    let Some(packages) = v.get("packages").and_then(Value::as_array) else {
        return Err("'packages' not found or is not an array in cargo metadata".into());
    };

    // Build what we expect for the "root" package's manifest path
    let root_manifest_path = workspace_root.join("Cargo.toml").to_string_lossy().to_string();

    // Try to find a package that matches the workspace root
    for pkg in packages {
        let pkg_manifest_path = pkg
            .get("manifest_path")
            .and_then(Value::as_str)
            .unwrap_or_default();

        if same_file_path(&pkg_manifest_path, &root_manifest_path) {
            // Found the root package
            let Some(pkg_name) = pkg.get("name").and_then(Value::as_str) else {
                return Err("Package in root has no 'name' in cargo metadata".into());
            };
            return Ok((workspace_root, pkg_name.to_owned()));
        }
    }

    // If we reach here, no package at the workspace root. Possibly a virtual manifest.
    // Fallback: if there's exactly one package total, pick it.
    if packages.len() == 1 {
        let pkg = &packages[0];
        let Some(pkg_obj) = pkg.as_object() else {
            return Err("Expected 'packages[0]' to be an object".into());
        };

        let Some(pkg_name) = pkg_obj.get("name").and_then(Value::as_str) else {
            return Err("Single package has no 'name' field".into());
        };
        let Some(pkg_manifest_str) = pkg_obj.get("manifest_path").and_then(Value::as_str) else {
            return Err("Single package has no 'manifest_path' field".into());
        };

        // We'll treat the parent directory of that single manifest as its root
        let package_path = PathBuf::from(pkg_manifest_str)
            .parent()
            .ok_or("Could not get parent directory of manifest_path")?
            .to_path_buf();

        return Ok((package_path, pkg_name.to_owned()));
    }

    // Otherwise, return an error if there's more than one package.
    Err(format!(
        "No package found in {} (virtual manifest?), and multiple packages exist; cannot pick a single fallback package.",
        root_manifest_path
    ))?
}

/// Compare two file paths in a slightly more robust way.
/// (On Windows, e.g., backslash vs forward slash).
fn same_file_path(a: &str, b: &str) -> bool {
    // Convert both to a canonical PathBuf
    let path_a = Path::new(a).components().collect::<Vec<_>>();
    let path_b = Path::new(b).components().collect::<Vec<_>>();
    path_a == path_b
}
