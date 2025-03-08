mod init;

use faasta_analyze::lint_project;
use anyhow::Error;
use reqwest::{multipart, Client};
use serde_json::Value;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use faasta_analyze::build_project;
use std::process::{exit, Command};
use std::{env, fmt};
use faasta_analyze::{analyze_cargo_file, analyze_rust_file};
use walkdir::WalkDir;
use zip::write::{ExtendedFileOptions, FileOptions};
use zip::{CompressionMethod, ZipWriter};

const UPLOAD_URL: &str = "http://127.0.0.1:8080/upload";
const INVOKE_URL: &str = "http://127.0.0.1:8080/";

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
/// Recursively walk the given `project_path` and upload all files (including `Cargo.toml`, `src/` content, etc.).
/// Each file is added to the multipart form with a field name that is the **relative path** from `project_path`.
/// Zips up the local project (skipping `target/` and build scripts)
/// and uploads it as a single multipart form field named `"archive"`.
pub async fn upload_project() -> Result<String, Error> {
    let (package_root, _package_name) = find_root_package().unwrap();

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

            if path.ends_with(".rs") {
                analyze_rust_file(&path.to_str().unwrap()).await?;
            }

            // TODO sanitizize cargo.tomls
            if path.file_name().and_then(|n| n.to_str()) == Some("Cargo.toml") {
                analyze_cargo_file(&path.to_str().unwrap())?;
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
        .file_name(format!("{}.zip", _package_name));

    let form = multipart::Form::new()
        .part("archive", zip_part);

    // 4) Send the POST request with our zip file in the form
    let url = format!("{}/{}", UPLOAD_URL, _package_name);
    let response = client
        .post(&url)
        .multipart(form)
        .send()
        .await?;

    // 5) Return the response body as text, or handle it however needed
    let text = response.text().await?;
    println!("Server response: {text}");
    println!("Function URL: {}", INVOKE_URL.to_string()  + &_package_name);

    Ok(text)
}


use clap::{Args, Parser, Subcommand};

/// Main entry point
#[tokio::main]
async fn main() {
    let Faasta::Faasta(cli) = Faasta::parse();

    match cli.command {
        Commands::Upload(_args) => {
            let spinner = indicatif::ProgressBar::new_spinner();
            spinner.set_message("Linting project...");
            spinner.enable_steady_tick(std::time::Duration::from_millis(100));

            lint_project(&env::current_dir().unwrap()).await.unwrap_or_else(|e| {
                spinner.finish_and_clear();
                eprintln!("Failed to lint project: {}", e);
                exit(1);
            });

            spinner.set_message("Uploading project...");
            upload_project().await.unwrap_or_else(|e| {
                spinner.finish_and_clear();
                eprintln!("Failed to upload project: {}", e);
                exit(1);
            });

            spinner.finish_and_clear();
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
        Commands::Build => {
            let (package_root, _package_name) = find_root_package().expect("Failed to find root package");

            // TODO Add safety /dependency lints here
            lint_project(&package_root).await.unwrap_or_else(|e| {
                eprintln!("Failed to lint project: {}", e);
                exit(1);
            });


            build_project(&env::current_dir().unwrap()).await.unwrap_or_else(|e| {
                eprintln!("Failed to build project: {}", e);
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
    /// Uploads a project to the server
    Upload(UploadArgs),
    /// Invokes a function with the specified name and argument
    Invoke(InvokeArgs),
    Init,
    New(NewArgs),
    Build,
}

#[derive(Args, Debug)]
struct UploadArgs {
    /// Path to the project to upload
    path: Option<String>,
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
        return Err("`cargo metadata` failed".into());
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
