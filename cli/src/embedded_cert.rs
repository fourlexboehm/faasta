//! This module contains code for TLS certificate handling.

use std::path::PathBuf;
use anyhow::{Context, Result};
use std::fs;
use std::io::Write;

/// Get the path to the certificate used for connecting to the server.
/// 
/// This function retrieves the server's certificate from the project root directory.
pub fn get_cert_path() -> Result<PathBuf> {
    // Get the faasta project root directory
    let project_root = find_faasta_project_root()?;
    
    // Create a temporary file for the certificate
    let temp_dir = std::env::temp_dir();
    let cert_path = temp_dir.join("faasta_cert.pem");
    
    // Read the certificate from the project root
    let source_cert_path = project_root.join("cert.pem");
    let cert_content = fs::read_to_string(&source_cert_path)
        .with_context(|| format!("Failed to read server certificate from {}", source_cert_path.display()))?;
    
    // Write the certificate to the temporary file
    let mut file = fs::File::create(&cert_path)
        .with_context(|| format!("Failed to create temporary certificate file at {}", cert_path.display()))?;
    file.write_all(cert_content.as_bytes())
        .context("Failed to write certificate to temporary file")?;
    
    Ok(cert_path)
}

/// Find the faasta project root directory
fn find_faasta_project_root() -> Result<PathBuf> {
    // Start from the current directory and look for a cargo workspace
    let current_dir = std::env::current_dir()
        .context("Failed to get current directory")?;
    
    // Try to find the project root by looking for a Cargo.toml file with faasta in the path
    let output = std::process::Command::new("cargo")
        .args(["locate-project", "--workspace", "--message-format=plain"])
        .output()
        .context("Failed to run cargo locate-project")?;
    
    if !output.status.success() {
        return Err(anyhow::anyhow!("Failed to locate project workspace"));
    }
    
    let cargo_toml_path = String::from_utf8(output.stdout)
        .context("Failed to parse cargo locate-project output")?
        .trim()
        .to_string();
    
    let project_dir = PathBuf::from(cargo_toml_path)
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Failed to get parent directory of Cargo.toml"))?
        .to_path_buf();
    
    Ok(project_dir)
}
