// Export modules
pub mod github_oauth;

use std::process::Command;
use std::path::{Path, PathBuf};
use serde_json::Value;

/// Find a workspace root package if it exists; otherwise pick the
/// current/only package from cargo metadata.
pub fn find_root_package() -> Result<(PathBuf, String, PathBuf), Box<dyn std::error::Error>> {
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

    // Extract target_directory
    let Some(target_dir_str) = v.get("target_directory").and_then(Value::as_str) else {
        return Err("No 'target_directory' found in cargo metadata".into());
    };
    let target_directory = PathBuf::from(target_dir_str);

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

        if same_file_path(pkg_manifest_path, &root_manifest_path) {
            // Found the root package
            let Some(pkg_name) = pkg.get("name").and_then(Value::as_str) else {
                return Err("Package in root has no 'name' in cargo metadata".into());
            };
            return Ok((workspace_root, pkg_name.to_owned(), target_directory));
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

        return Ok((package_path, pkg_name.to_owned(), target_directory));
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
