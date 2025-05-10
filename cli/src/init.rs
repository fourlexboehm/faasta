use clap::Args;
use std::error::Error;
use std::path::Path;
use std::{env, fs, io};

/// CLI arguments for the `init` command
#[derive(Args, Debug)]
pub struct NewArgs {
    /// The name of the package to create
    pub package_name: String,
}

pub const HTTP_CARGO_TOML: &str = include_str!("../template/notCargo.toml");
pub const HTTP_LIB_RS: &str = include_str!("../template/lib.rs");
pub fn handle_new(args: &NewArgs) -> Result<(), Box<dyn Error>> {
    dbg!(&args);
    let current_dir = env::current_dir()?;
    let new_project_dir = current_dir.join(&args.package_name);

    if new_project_dir.exists() && !args.package_name.is_empty() {
        return Err(format!("Directory '{}' already exists", args.package_name).into());
    }
    if new_project_dir.join("Cargo.toml").exists() {
        return Err(format!(
            "Cargo.toml already exists in '{}'",
            new_project_dir.display()
        )
        .into());
    }
    fs::create_dir_all(new_project_dir.join("src"))?;
    let pkg_name = if args.package_name.is_empty() {
        "axum_serverless"
    } else {
        &*args.package_name
    };

    write_files(&new_project_dir, HTTP_CARGO_TOML, HTTP_LIB_RS, pkg_name)?;

    println!(
        "Successfully created new Axum project '{}' at '{}'",
        args.package_name,
        new_project_dir.display()
    );
    Ok(())
}

/// Writes the embedded Cargo.toml & main.rs to disk,
/// updating the `[package] name` in Cargo.toml to `package_name`.
fn write_files(
    project_dir: &Path,
    cargo_toml_str: &str,
    main_rs_str: &str,
    package_name: &str,
) -> io::Result<()> {
    // 1. Write Cargo.toml
    // EVENT_PATHS

    let cargo_toml_path = project_dir.join("Cargo.toml");
    let updated_cargo_toml = rewrite_package_name(cargo_toml_str, package_name);
    fs::write(cargo_toml_path, updated_cargo_toml)?;

    // 2. Write src/lib.rs
    let main_rs_path = project_dir.join("src").join("lib.rs");
    fs::write(main_rs_path, main_rs_str)?;

    Ok(())
}

/// Replaces the line `name = "whatever"` inside `[package]` with the user-provided `package_name`.
fn rewrite_package_name(toml_input: &str, package_name: &str) -> String {
    let mut in_package = false;
    let mut output = String::new();

    for line in toml_input.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("[package]") {
            in_package = true;
            output.push_str(line);
            output.push('\n');
            continue;
        }

        if in_package && trimmed.starts_with("name =") {
            output.push_str(&format!("name = \"{package_name}\"\n"));
            in_package = false; // Only replace once
            continue;
        }

        output.push_str(line);
        output.push('\n');
    }

    output
}
