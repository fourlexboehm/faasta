// Tests for the CLI's build command
use anyhow::Result;
use tempfile::TempDir;
use std::fs;
use std::path::PathBuf;
use faasta_analyze::{lint_project, build_project};

/// Unit tests for the `cargo faasta build` command
#[cfg(test)]
mod tests {
    use super::*;
    // For async tests
    
    /// Create a valid FaaSta function project structure for testing
    fn create_valid_function_project() -> Result<(TempDir, PathBuf)> {
        let temp_dir = TempDir::new()?;
        let project_path = temp_dir.path().to_path_buf();
        
        // Create src directory and lib.rs
        fs::create_dir_all(&project_path.join("src"))?;
        
        // First, we need to create a simple function in lib.rs that doesn't use macros
        // This will avoid the dependency issues during tests
        fs::write(
            project_path.join("src").join("lib.rs"),
            r#"
use axum::body::{Body, Bytes};
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::Response;
use cap_async_std::fs::Dir;

// Simple function without the faasta macro for testing
pub async fn handler(_method: Method, uri: Uri, _headers: HeaderMap, _body: Bytes, _dir: Dir) -> Response<Body> {
    let path = uri.path();
    
    if path == "/hello" {
        return Response::builder()
            .status(StatusCode::OK)
            .body(Body::from("Hello, World!"))
            .unwrap();
    }
    
    Response::builder()
        .status(StatusCode::OK)
        .body(Body::from(format!("Got request to {}", path)))
        .unwrap()
}
"#,
        )?;
        
        // Create Cargo.toml with allowed dependencies
        fs::write(
            project_path.join("Cargo.toml"),
            r#"
[package]
name = "test-function"
version = "0.1.0"
edition = "2021"
description = "A test FaaSta function"
authors = ["Test User <test@example.com>"]

[lib]
path = "src/lib.rs"

[dependencies]
axum = "0.7"
cap-async-std = "3.4"
"#,
        )?;
        
        // Create a Clippy.toml file with security rules
        fs::write(
            project_path.join("Clippy.toml"),
            r#"
# Explicitly disallow certain calls
disallowed-methods = [
    # Filesystem access
    { path = "std::fs", reason = "Use cap-fs instead for safer file operations" },
    # Network access
    { path = "std::net", reason = "Use cap-net instead for controlled network access" },
    # Process spawning
    { path = "std::process", reason = "Process spawning is not allowed" },
]
"#,
        )?;
        
        Ok((temp_dir, project_path))
    }
    
    /// Create an insecure project with restricted APIs
    fn create_insecure_function_project() -> Result<(TempDir, PathBuf)> {
        let temp_dir = TempDir::new()?;
        let project_path = temp_dir.path().to_path_buf();
        
        // Create src directory and lib.rs
        fs::create_dir_all(&project_path.join("src"))?;
        
        // Create an insecure lib.rs using unsafe APIs
        fs::write(
            project_path.join("src").join("lib.rs"),
            r#"
use axum::body::{Body, Bytes};
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::Response;
use cap_async_std::fs::Dir;
use std::fs;  // Using restricted std::fs

// Insecure function for testing
pub async fn handler(_method: Method, uri: Uri, _headers: HeaderMap, body: Bytes, _dir: Dir) -> Response<Body> {
    let path = uri.path();
    
    // Insecure file access
    if path == "/read" {
        let file_path = String::from_utf8(body.to_vec()).unwrap_or_default();
        let content = fs::read_to_string(file_path).unwrap_or_else(|_| "Failed to read".to_string());
        
        return Response::builder()
            .status(StatusCode::OK)
            .body(Body::from(content))
            .unwrap();
    }
    
    Response::builder()
        .status(StatusCode::OK)
        .body(Body::from("Insecure function"))
        .unwrap()
}
"#,
        )?;
        
        // Create Cargo.toml
        fs::write(
            project_path.join("Cargo.toml"),
            r#"
[package]
name = "test-insecure-function"
version = "0.1.0"
edition = "2021"
description = "A test insecure FaaSta function"
authors = ["Test User <test@example.com>"]

[lib]
path = "src/lib.rs"

[dependencies]
axum = "0.7"
cap-async-std = "3.4"
"#,
        )?;
        
        // Create a Clippy.toml file
        fs::write(
            project_path.join("Clippy.toml"),
            r#"
# Explicitly disallow certain calls
disallowed-methods = [
    # Filesystem access
    { path = "std::fs", reason = "Use cap-fs instead for safer file operations" },
    # Network access
    { path = "std::net", reason = "Use cap-net instead for controlled network access" },
    # Process spawning
    { path = "std::process", reason = "Process spawning is not allowed" },
]
"#,
        )?;
        
        Ok((temp_dir, project_path))
    }

    #[tokio::test]
    async fn test_lint_valid_function() -> Result<()> {
        let (_temp_dir, project_path) = create_valid_function_project()?;
        
        // This should pass without error since the function is valid
        lint_project(&project_path).await?;
        
        Ok(())
    }
    
    #[tokio::test]
    async fn test_lint_insecure_function() -> Result<()> {
        let (_temp_dir, project_path) = create_insecure_function_project()?;
        
        // This should fail due to security violations
        let result = lint_project(&project_path).await;
        assert!(result.is_err());
        
        // The error should mention security issues
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("security") || 
            err.contains("unsafe") || 
            err.contains("std::fs"),
            "Error message didn't mention security issues: {}", err
        );
        
        Ok(())
    }

    #[tokio::test]
    async fn test_build_valid_function() -> Result<()> {
        let (_temp_dir, project_path) = create_valid_function_project()?;
        
        // This should pass since the function is valid
        // Note that build_project calls lint_project internally
        let result = build_project(&project_path).await;
        
        // Print detailed error if the build fails
        if let Err(ref e) = result {
            eprintln!("Build failed: {}", e);
        }
        
        result?;
        
        Ok(())
    }
    
    #[tokio::test]
    async fn test_build_insecure_function() -> Result<()> {
        let (_temp_dir, project_path) = create_insecure_function_project()?;
        
        // This should fail due to security violations caught by lint_project
        let result = build_project(&project_path).await;
        assert!(result.is_err());
        
        // The error should mention security issues
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("security") || 
            err.contains("unsafe") || 
            err.contains("std::fs"),
            "Error message didn't mention security issues: {}", err
        );
        
        Ok(())
    }

    #[tokio::test]
    async fn test_missing_librs() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let project_path = temp_dir.path().to_path_buf();
        
        // Create an incomplete project (missing lib.rs)
        fs::create_dir_all(&project_path.join("src"))?;
        
        fs::write(
            project_path.join("Cargo.toml"),
            r#"
[package]
name = "test-missing-librs"
version = "0.1.0"
edition = "2021"

[lib]
path = "src/lib.rs"

[dependencies]
axum = "0.7"
"#,
        )?;
        
        // Linting might actually fail since clippy needs the lib.rs file
        // So we should handle both cases - either linting fails (which is fine)
        // or it passes (also fine, as it's just checking security issues)
        let _lint_result = lint_project(&project_path).await;
        
        // We don't assert on the lint result - it could pass or fail
        
        // Build should definitely fail because we check for lib.rs
        let build_result = build_project(&project_path).await;
        assert!(build_result.is_err());
        
        Ok(())
    }
}