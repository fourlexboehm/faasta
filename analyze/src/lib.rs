
use anyhow::Result;
use geiger::{find_unsafe_in_string, IncludeTests};
use lazy_static::lazy_static;
use std::collections::HashSet;
use std::fs;
use tokio::process::Command;
use syn::{visit::Visit, Attribute, Path};
use log::info;

static CLIPPY: &str = include_str!("../Clippy.toml");

struct StdAndNoMangleVisitor {
    has_std_usage: bool,
    has_no_mangle: bool,
}

impl<'ast> Visit<'ast> for StdAndNoMangleVisitor {
    fn visit_attribute(&mut self, attr: &'ast Attribute) {
        // Check if the attribute is `#[no_mangle]`
        if attr.path().is_ident("no_mangle") {
            self.has_no_mangle = true;
        }
        syn::visit::visit_attribute(self, attr);
    }

    fn visit_path(&mut self, path: &'ast Path) {
        // Check if the path starts with `std`
        if path.segments.first().map(|segment| segment.ident == "std").unwrap_or(false) {
            self.has_std_usage = true;
        }
        syn::visit::visit_path(self, path);
    }
}

pub async fn analyze_rust_file(file_path: &str) -> Result<()> {
    if !file_path.ends_with(".rs") {
        return Ok(());
    }

    // Run rustc to get the expanded code
    // let output = Command::new("rustc")
    //     .arg(file_path)
    //     .arg("-Zunpretty=expanded")
    //     .arg("--edition")
    //     .arg("2021")
    //     .output()?;
    let output = Command::new("cargo")
        .arg("expand")
        .arg("--ugly")
        .current_dir("../".to_string() + file_path)
        .output().await?;

    // println!("{}", output)
    println!("expanded");

    if !output.status.success() {
        return Err(anyhow::anyhow!(
            "Failed to expand the code in {}: {}",
            file_path,
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let expanded_code = String::from_utf8(output.stdout)?;

    dbg!(&expanded_code);
    // Analyze the expanded code using geiger
    let geiger_report = find_unsafe_in_string(&expanded_code, IncludeTests::No)?;
    dbg!(&geiger_report);

    println!("geiger ran");
    // Check for unsafe code
    if !geiger_report.forbids_unsafe && geiger_report.counters.has_unsafe() {
        return Err(anyhow::anyhow!(
            "Unsafe code found in {}, this is not supported",
            file_path
        ));
    }

    // Parse the source code into a syntax tree
    let syntax_tree: syn::File = syn::parse_str(&expanded_code)?;

    // Create a visitor to search for `std::` and `#[no_mangle]`
    let mut visitor = StdAndNoMangleVisitor {
        has_std_usage: false,
        has_no_mangle: false,
    };

    // Walk the syntax tree
    visitor.visit_file(&syntax_tree);

    // Report findings
    if visitor.has_std_usage {
        return Err(anyhow::anyhow!(
            "The file {} uses `std::`, this is not allowed",
            file_path
        ));
    }

    // if visitor.has_no_mangle {
    //     println!("The file {} contains `#[no_mangle]` attributes.", file_path);
    // }
    //
    Ok(())
}

pub fn analyze_cargo_file(file_path: &str) -> Result<()> {
    // Read the Cargo.toml file
    // println!("parsing toml");
    info!("Validating Cargo.toml file");
    let cargo_toml = fs::read_to_string(file_path)?;

    // Parse the Cargo.toml file
    let cargo_toml: toml::Value = toml::from_str(&cargo_toml)?;

    // Check for dependencies that are not in the allowlist
    if let Some(deps) = cargo_toml.get("dependencies") {
        if let Some(deps) = deps.as_table() {
            for (dep, _) in deps {
                if !ALLOWLIST.contains(dep) {
                    return Err(anyhow::anyhow!(
                        "Dependency {} is not in the allowlist",
                        dep
                    ));
                }
            }
        }
    }

    Ok(())
}

pub async fn lint_project(project_dir: &std::path::Path) -> std::result::Result<(), anyhow::Error> {
    // write the clippy config to a file
    let clippy_toml_path = project_dir.join("Clippy.toml");
    fs::write(clippy_toml_path, CLIPPY)?;
    let clippy_output = Command::new("cargo")
        .arg("clippy")
        .arg("--")
        .arg("-D")
        .arg("warnings") // Treat warnings as errors
        .current_dir(project_dir)
        .output()
        .await?;

    if !clippy_output.status.success() {
        let stderr = String::from_utf8_lossy(&clippy_output.stderr);
        return Err(anyhow::anyhow!("Cargo clippy failed: {}", stderr));
    }

    Ok(())
}
/// Runs `cargo build --release` in the specified project directory.
pub async fn build_project(project_dir: &std::path::Path) -> std::result::Result<(), anyhow::Error> {
    let output = Command::new("cargo")
        .arg("build")
        .arg("--release")
        .arg("--target-dir")
        .arg("../target")
        .env("FAASTA_HMAC_SECRET",  include_str!("../../faasta-hmac-secret"))
        .current_dir(project_dir)
        .output()
        .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("Cargo build failed: {}", stderr));
    }

    Ok(())
}



lazy_static! {
    static ref ALLOWLIST: HashSet<String> = {
        vec![
            // "tokio".to_string(),
            "rust-s3".to_string(),
            "aws-sdk-s3".to_string(),
            "axum".to_string(),
            "serde".to_string(),
            "cap-async-std".to_string(),
            "serde_json".to_string(),
            "reqwest".to_string(),
            "tokio-util".to_string(),
            "tokio-tungstenite".to_string(),
            "tokio-rustls".to_string(),
            "sqlx".to_string(),
            "cap-async".to_string(),
            "uuid".to_string(),
            "macros".to_string(),
            "faasta-macros".to_string(),
        ]
        .into_iter()
        .collect()
    };
}