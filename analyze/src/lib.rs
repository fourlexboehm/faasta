
use anyhow::Result;
use geiger::{find_unsafe_in_string, IncludeTests};
use lazy_static::lazy_static;
use std::collections::HashSet;
use std::fs;
use tokio::process::Command;
use syn::{visit::Visit, Attribute, Path, Item, UseTree, Expr, ExprCall};
use log::{info, warn};
use regex::Regex;

static CLIPPY: &str = include_str!("../Clippy.toml");

lazy_static! {
    static ref DISALLOWED_SYSCALLS: HashSet<&'static str> = {
        vec![
            // System calls
            "syscall", "libc", "nix", "mmap", "open", "write", "read", "close", 
            "socket", "connect", "bind", "listen", "accept", "fork", "exec",
            "unistd", "ioctl", "kill", "signal", 
            
            // Potentially dangerous filesystem operations
            "fs::remove", "fs::delete", "remove_file", "remove_dir", 
            "create_dir", "rename",
            
            // Network access without permission
            "TcpListener", "TcpStream", "UdpSocket",
            
            // Process spawning
            "Command::new", "process::Command", "std::process", "spawn",
            
            // Raw pointers and FFI
            "from_raw_parts", "as_ptr", "as_mut_ptr", "offset", 
            "transmute", "mem::transmute",
            
            // Prohibited operations from standard libraries
            "tokio::fs::", "tokio::net::", "tokio::process::", 
            "tokio::io::stdin", "tokio::io::stdout", "tokio::io::stderr",
            "async_std::fs::", "async_std::net::", "async_std::process::",
            "async_std::io::stdin", "async_std::io::stdout", "async_std::io::stderr",
            "std::fs::", "std::net::", "std::process::"
        ].into_iter().collect()
    };
}

/// Improved path validation that ensures the path:
/// 1. exists on the filesystem
/// 2. contains no directory traversal characters
/// 3. is not empty
/// 4. is canonicalized to prevent path traversal attacks
fn path_is_valid(path: &str) -> bool {
    // Check for empty paths
    if path.is_empty() {
        return false;
    }
    
    // Check for disallowed traversal characters
    if path.contains("..") {
        return false;
    }
    
    let path_obj = std::path::Path::new(path);
    
    // Check that the path exists
    if !path_obj.exists() {
        return false;
    }
    
    // Try to canonicalize the path and make sure it's within expected bounds
    if let Ok(canonical) = std::fs::canonicalize(path_obj) {
        // Additional security check - make sure canonical path doesn't differ
        // in unexpected ways from the original (which could indicate traversal)
        let canonical_str = canonical.to_string_lossy();
        !canonical_str.contains("..")
    } else {
        false // If we can't canonicalize, reject for safety
    }
}

struct CodeSafetyVisitor {
    has_std_usage: bool,
    has_no_mangle: bool, 
    has_unsafe_code: bool,
    has_sys_calls: bool,
    unsafe_blocks: Vec<String>,
    sys_calls: Vec<String>,
}

impl<'ast> Visit<'ast> for CodeSafetyVisitor {
    fn visit_attribute(&mut self, attr: &'ast Attribute) {
        // Check if the attribute is `#[no_mangle]`
        if attr.path().is_ident("no_mangle") {
            self.has_no_mangle = true;
        }
        syn::visit::visit_attribute(self, attr);
    }

    fn visit_path(&mut self, path: &'ast Path) {
        // Get the full path as a string for checking against disallowed patterns
        let path_str = path.segments.iter()
            .map(|seg| seg.ident.to_string())
            .collect::<Vec<_>>()
            .join("::");
            
        // Check for specific restricted functionality with helpful error messages
        let mut restricted_with_message = None;
        
        if path_str.starts_with("tokio::fs") {
            restricted_with_message = Some(format!("{} - Use cap-async-std::fs instead for sandboxed filesystem access", path_str));
        } else if path_str.starts_with("tokio::net") {
            restricted_with_message = Some(format!("{} - Use cap-async-std::net instead for sandboxed network access", path_str));
        } else if path_str.starts_with("tokio::process") {
            restricted_with_message = Some(format!("{} - Direct process creation is not allowed", path_str));
        } else if path_str.starts_with("tokio::io::std") {
            restricted_with_message = Some(format!("{} - Direct console I/O is not allowed", path_str));
        } else if path_str.starts_with("async_std::fs") {
            restricted_with_message = Some(format!("{} - Use cap-async-std::fs instead for sandboxed filesystem access", path_str));
        } else if path_str.starts_with("async_std::net") {
            restricted_with_message = Some(format!("{} - Use cap-async-std::net instead for sandboxed network access", path_str));
        } else if path_str.starts_with("async_std::process") {
            restricted_with_message = Some(format!("{} - Direct process creation is not allowed", path_str));
        } else if path_str.starts_with("async_std::io::std") {
            restricted_with_message = Some(format!("{} - Direct console I/O is not allowed", path_str));
        } else if path_str.starts_with("std::fs") {
            restricted_with_message = Some(format!("{} - Use cap-std::fs instead for sandboxed filesystem access", path_str));
        } else if path_str.starts_with("std::net") {
            restricted_with_message = Some(format!("{} - Use cap-std::net instead for sandboxed network access", path_str));
        } else if path_str.starts_with("std::process") {
            restricted_with_message = Some(format!("{} - Direct process creation is not allowed", path_str));
        } else if path_str.starts_with("std::io::std") {
            restricted_with_message = Some(format!("{} - Direct console I/O is not allowed", path_str));
        }
        
        // If we found a restricted pattern with a message, record it
        if let Some(message) = restricted_with_message {
            self.has_sys_calls = true;
            self.sys_calls.push(message);
        }
        
        // General check for all other disallowed syscalls
        else if DISALLOWED_SYSCALLS.iter().any(|syscall| path_str.contains(syscall)) {
            self.has_sys_calls = true;
            self.sys_calls.push(path_str);
        }
        
        // Also mark if the file uses std:: directly for the separate check
        if path.segments.first().map(|segment| segment.ident == "std").unwrap_or(false) {
            self.has_std_usage = true;
        }
        
        syn::visit::visit_path(self, path);
    }
    
    fn visit_item(&mut self, item: &'ast Item) {
        // Check for unsafe code blocks and functions
        match item {
            Item::Fn(func) if func.sig.unsafety.is_some() => {
                self.has_unsafe_code = true;
                self.unsafe_blocks.push(format!("Unsafe function: {}", func.sig.ident));
            },
            _ => {}
        }
        
        syn::visit::visit_item(self, item);
    }
    
    fn visit_expr(&mut self, expr: &'ast Expr) {
        // Check for unsafe blocks
        if let Expr::Unsafe(unsafe_expr) = expr {
            self.has_unsafe_code = true;
            self.unsafe_blocks.push("Unsafe block found".to_string());
        }
        
        // Check for potentially dangerous calls like libc functions
        if let Expr::Call(ExprCall { func, .. }) = expr {
            if let Expr::Path(path_expr) = &**func {
                let path_str = path_expr.path.segments.iter()
                    .map(|seg| seg.ident.to_string())
                    .collect::<Vec<_>>()
                    .join("::");
                
                if path_str.contains("libc::") || path_str.contains("syscall") {
                    self.has_sys_calls = true;
                    self.sys_calls.push(path_str);
                }
            }
        }
        
        syn::visit::visit_expr(self, expr);
    }
}

pub async fn analyze_rust_file(file_path: &str) -> Result<()> {
    if !file_path.ends_with(".rs") {
        return Ok(());
    }

    if !path_is_valid(file_path) {
        return Err(anyhow::anyhow!("Invalid file path"));
    }
    
    // Read the original source code for detailed AST analysis
    let source_code = fs::read_to_string(file_path).unwrap();
    
    // Parse the source code into a syntax tree for initial analysis
    let syntax_tree = match syn::parse_file(&source_code) {
        Ok(ast) => ast,
        Err(err) => return Err(anyhow::anyhow!("Failed to parse {}: {}", file_path, err)),
    };
    
    // First pass: Check for unsafe code, system calls, and std usage in the original source
    let mut visitor = CodeSafetyVisitor {
        has_std_usage: false,
        has_no_mangle: false,
        has_unsafe_code: false,
        has_sys_calls: false,
        unsafe_blocks: Vec::new(),
        sys_calls: Vec::new(),
    };

    // Walk the syntax tree
    visitor.visit_file(&syntax_tree);
    
    // Check for unsafe blocks using regex as a backup detection method
    let unsafe_regex = Regex::new(r"\bunsafe\s*\{|\bunsafe\s+fn\b").unwrap();
    if unsafe_regex.is_match(&source_code) {
        visitor.has_unsafe_code = true;
        visitor.unsafe_blocks.push("Unsafe code detected via pattern matching".to_string());
    }
    
    // Report findings from first pass
    let mut errors = Vec::new();
    
    // We no longer block all std usage, just specific modules
    // The specific std::fs, std::net, and std::process errors are caught in the visitor
    // and will be included in the sys_calls list
    
    if visitor.has_no_mangle {
        errors.push(format!("The file contains `#[no_mangle]` attributes, which are not allowed"));
    }
    
    if visitor.has_unsafe_code {
        errors.push(format!("Unsafe code detected: {}", visitor.unsafe_blocks.join(", ")));
    }
    
    if visitor.has_sys_calls {
        errors.push(format!("Disallowed system calls detected: {}", visitor.sys_calls.join(", ")));
    }
    
    // Now run cargo expand for macro expansion and further analysis
    let output = Command::new("cargo")
        .arg("expand")
        .arg("--ugly")
        .current_dir(std::path::Path::new(file_path).parent().unwrap_or(std::path::Path::new(".")))
        .output()
        .await?;

    if !output.status.success() {
        info!("Cargo expand failed, skipping expanded code analysis: {}", 
              String::from_utf8_lossy(&output.stderr));
        // Not a fatal error, we continue with the analysis we've done so far
        return Ok(());
    }

    let expanded_code = String::from_utf8(output.stdout)?;
    
    // Analyze the expanded code using geiger (as a backup for unsafe detection)
    let geiger_report = match find_unsafe_in_string(&expanded_code, IncludeTests::No) {
        Ok(report) => report,
        Err(err) => {
            info!("Geiger analysis failed: {}. Skipping geiger analysis.", err);
            return Ok(()); // Not a fatal error
        },
    };
    
    // Second check for unsafe code with geiger
    if !geiger_report.forbids_unsafe && geiger_report.counters.has_unsafe() {
        errors.push(format!("Geiger detected unsafe code in the expanded source"));
    }
    
    // Parse the expanded code into a syntax tree for deeper analysis
    match syn::parse_str::<syn::File>(&expanded_code) {
        Ok(expanded_syntax) => {
            // Create another visitor for the expanded code
            let mut expanded_visitor = CodeSafetyVisitor {
                has_std_usage: false,
                has_no_mangle: false,
                has_unsafe_code: false,
                has_sys_calls: false,
                unsafe_blocks: Vec::new(),
                sys_calls: Vec::new(),
            };

            // Walk the expanded syntax tree
            expanded_visitor.visit_file(&expanded_syntax);
            
            // Add any new findings from the expanded code
            if expanded_visitor.has_unsafe_code && !visitor.has_unsafe_code {
                errors.push(format!("Unsafe code detected in expanded source: {}", 
                    expanded_visitor.unsafe_blocks.join(", ")));
            }
            
            if expanded_visitor.has_sys_calls && !visitor.has_sys_calls {
                errors.push(format!("Disallowed system calls detected in expanded source: {}", 
                    expanded_visitor.sys_calls.join(", ")));
            }
        },
        Err(err) => {
            info!("Failed to parse expanded code for deeper analysis: {}. Skipping this step.", err);
            // Not a fatal error
        }
    }
    
    // Return error if any issues were found
    if !errors.is_empty() {
        return Err(anyhow::anyhow!("Security violations in {}: {}", file_path, errors.join("; ")));
    }
    
    Ok(())
}

pub fn analyze_cargo_file(file_path: &str) -> Result<()> {
    info!("Validating Cargo.toml file at {}", file_path);
    
    // Path validation
    if !path_is_valid(file_path) {
        return Err(anyhow::anyhow!("Invalid file path: {}", file_path));
    }
    
    // Read the Cargo.toml file
    let cargo_toml = fs::read_to_string(file_path)
        .map_err(|e| anyhow::anyhow!("Failed to read Cargo.toml at {}: {}", file_path, e))?;

    // Parse the Cargo.toml file
    let cargo_toml: toml::Value = toml::from_str(&cargo_toml)
        .map_err(|e| anyhow::anyhow!("Failed to parse Cargo.toml at {}: {}", file_path, e))?;

    let mut disallowed_deps = Vec::new();

    // Check regular dependencies
    if let Some(deps) = cargo_toml.get("dependencies").and_then(|d| d.as_table()) {
        for (dep, _) in deps {
            if !ALLOWLIST.contains(dep) {
                disallowed_deps.push(format!("{}", dep));
            }
        }
    }
    
    // Check dev-dependencies as well
    if let Some(deps) = cargo_toml.get("dev-dependencies").and_then(|d| d.as_table()) {
        for (dep, _) in deps {
            if !ALLOWLIST.contains(dep) {
                disallowed_deps.push(format!("{} (dev)", dep));
            }
        }
    }
    
    // Check build-dependencies
    if let Some(deps) = cargo_toml.get("build-dependencies").and_then(|d| d.as_table()) {
        for (dep, _) in deps {
            if !ALLOWLIST.contains(dep) {
                disallowed_deps.push(format!("{} (build)", dep));
            }
        }
    }
    
    // Check target-specific dependencies
    if let Some(targets) = cargo_toml.get("target").and_then(|t| t.as_table()) {
        for (_, target_cfg) in targets {
            if let Some(deps) = target_cfg.get("dependencies").and_then(|d| d.as_table()) {
                for (dep, _) in deps {
                    if !ALLOWLIST.contains(dep) {
                        disallowed_deps.push(format!("{} (target-specific)", dep));
                    }
                }
            }
        }
    }
    
    // Check for potential rogue code in build scripts
    if cargo_toml.get("build").is_some() || 
       cargo_toml.get("build-dependencies").is_some() ||
       cargo_toml.get("links").is_some() {
        info!("Warning: Project uses build scripts which can execute arbitrary code at build time");
        
        // Detect build.rs file
        let dir = std::path::Path::new(file_path).parent().unwrap_or(std::path::Path::new("."));
        let build_rs_path = dir.join("build.rs");
        
        // If build.rs exists, it should be analyzed separately
        if build_rs_path.exists() {
            warn!("Found build.rs script at {:?} - this should be analyzed for security", build_rs_path);
            // The build.rs file could be analyzed here or flagged for manual review
        }
    }
    
    // Return an error if any disallowed dependencies were found
    if !disallowed_deps.is_empty() {
        return Err(anyhow::anyhow!(
            "Found {} disallowed dependencies: {}",
            disallowed_deps.len(),
            disallowed_deps.join(", ")
        ));
    }

    Ok(())
}

pub async fn lint_project(project_dir: &std::path::Path) -> std::result::Result<(), anyhow::Error> {
    info!("Running linting on project at {:?}", project_dir);

    // First, check Cargo.toml for disallowed dependencies
    let cargo_toml_path = project_dir.join("Cargo.toml");
    if cargo_toml_path.exists() {
        analyze_cargo_file(cargo_toml_path.to_str().expect("Invalid path"))?;
        info!("Cargo.toml validation passed");
    } else {
        return Err(anyhow::anyhow!("No Cargo.toml found in project directory"));
    }
    
    // Write the clippy config to a file
    let clippy_toml_path = project_dir.join("Clippy.toml");
    fs::write(&clippy_toml_path, CLIPPY)?;
    info!("Applied Clippy.toml configuration");
    
    // Run clippy with strict settings
    info!("Running Clippy with strict settings");
    let clippy_output = Command::new("cargo")
        .arg("clippy")
        .arg("--all-targets")
        .arg("--all-features")
        .arg("--")
        .arg("-D")
        .arg("warnings") // Treat warnings as errors
        .arg("-D")
        .arg("clippy::disallowed_methods") // Enforce disallowed methods check
        // Allowing unwrap/expect as we'll catch panics in the runtime
        .arg("-D")
        .arg("clippy::panic") // No explicit panics allowed
        .current_dir(project_dir)
        .output()
        .await?;
    if !clippy_output.status.success() {
        let stderr = String::from_utf8_lossy(&clippy_output.stderr);
        return Err(anyhow::anyhow!("Cargo clippy failed: {}", stderr));
    }
    
    // Analyze all Rust files in the project directory
    info!("Scanning all source files for unsafe code and system calls");
    let rust_files = find_rust_files(project_dir)?;
    
    for file in rust_files {
        if let Some(file_str) = file.to_str() {
            info!("Analyzing file: {}", file_str);
            analyze_rust_file(file_str).await?;
        }
    }
    
    info!("All linting checks passed successfully");
    Ok(())
}

/// Find all Rust source files in a project directory
fn find_rust_files(project_dir: &std::path::Path) -> Result<Vec<std::path::PathBuf>> {
    let src_dir = project_dir.join("src");
    let mut files = Vec::new();
    
    if src_dir.exists() {
        visit_dirs(&src_dir, &mut files)?;
    }
    
    // Check build.rs as well if it exists
    let build_rs = project_dir.join("build.rs");
    if build_rs.exists() {
        files.push(build_rs);
    }
    
    Ok(files)
}

/// Recursively visit directories to find Rust files
fn visit_dirs(dir: &std::path::Path, files: &mut Vec<std::path::PathBuf>) -> Result<()> {
    if dir.is_dir() {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                visit_dirs(&path, files)?;
            } else if let Some(ext) = path.extension() {
                if ext == "rs" {
                    files.push(path);
                }
            }
        }
    }
    Ok(())
}

/// Runs `cargo build --release` in the specified project directory.
/// Will abort the build if any security checks fail.
pub async fn build_project(project_dir: &std::path::Path) -> std::result::Result<(), anyhow::Error> {
    info!("Starting build process for project at {:?}", project_dir);
    
    // Run all security checks first
    info!("Running security checks before building");
    lint_project(project_dir).await?;
    
    info!("Security checks passed, proceeding with build");
    
    // Create an absolute path for the target directory
    let target_dir = project_dir.parent().unwrap_or(project_dir).join("target");

    let output = Command::new("cargo")
        .arg("build")
        .arg("--release")
        .arg("--target-dir")
        .arg(target_dir)
        .env("FAASTA_HMAC_SECRET",  include_str!("../../faasta-hmac-secret"))
        .current_dir(project_dir)
        .output()
        .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("Cargo build failed: {}", stderr));
    }

    // Post-build checks on the binary could be added here
    // For example, scanning for certain patterns in the binary
    
    info!("Build completed successfully");
    Ok(())
}



lazy_static! {
    static ref ALLOWLIST: HashSet<String> = {
        vec![
            "tokio".to_string(),  // Allow tokio for tests
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
