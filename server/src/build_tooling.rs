use crate::LIB_CACHE;
use axum::extract::Path as AxumPath;
use axum::{extract::Multipart, http::StatusCode, response::IntoResponse};
use faasta_analyze::{build_project, lint_project};
use futures::{TryFutureExt, TryStreamExt};
use std::io::Read;
use std::path::Path as StdPath;
use std::{
    io,
    path::{Path, PathBuf},
};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use tokio::io::AsyncReadExt;
use tokio::{
    fs::{self, File},
    io::BufWriter,
    process::Command,
};
use tokio_util::io::StreamReader;
use zip::ZipArchive;

/// Handle uploading multiple files via multipart and then `cargo build` them.
/// Expects a Cargo project structure: Cargo.toml, src/, etc.
/// Directory where builds are stored
const BUILDS_DIRECTORY: &str = "./builds";

/// Handler that accepts a ZIP archive, extracts it, and builds the Cargo project.
pub async fn handle_upload_and_build(
    AxumPath(function_name): AxumPath<String>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    dbg!(&function_name);

    let secret = include_str!("../../faasta-hmac-secret");
    // Create a unique project directory based on the function name
    let project_dir = PathBuf::from(BUILDS_DIRECTORY).join(&function_name);
    let hmac = generate_hmac(&function_name, secret);

    // Ensure the project directory exists
    if let Err(e) = fs::create_dir_all(&project_dir).await {
        eprintln!("Failed to create build directory: {}", e);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to create build directory".to_owned(),
        )
            .into_response();
    }

    // Process each field in the multipart form
    let Some(field_result) = multipart.next_field().await.transpose() else {
        return (StatusCode::BAD_REQUEST, "No files uploaded").into_response();
    };
    match field_result {
        Ok(field) => {
            // Expect a single field named "archive"
            if field.name() != Some("archive") {
                eprintln!("Unexpected field: {:?}", field.name());
            }

            if let Some(filename) = field.file_name() {
                if !filename.ends_with(".zip") {
                    eprintln!("Uploaded file is not a ZIP archive: {}", filename);
                    return (
                        StatusCode::BAD_REQUEST,
                        "Uploaded file must be a ZIP archive".to_owned(),
                    )
                        .into_response();
                }
            } else {
                eprintln!("Uploaded archive has no filename");
                return (
                    StatusCode::BAD_REQUEST,
                    "Uploaded archive must have a filename".to_owned(),
                )
                    .into_response();
            }

            // Save the uploaded ZIP archive to disk
            let zip_path = project_dir.join("project.zip");
            if let Err(e) = save_field_to_file(&zip_path, field).await {
                eprintln!("Failed to save uploaded ZIP: {}", e);
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Failed to save uploaded ZIP".to_owned(),
                )
                    .into_response();
            }

            match fs::metadata(&zip_path).await {
                Ok(metadata) => {
                    if metadata.len() > 1_048_576 {
                        // 1 MB in bytes
                        let _ = fs::remove_file(&zip_path).await;
                        return (
                            StatusCode::BAD_REQUEST,
                            "Maximum 1 MB Zip File size".to_string(),
                        )
                            .into_response();
                    }
                }
                Err(e) => {
                    eprintln!("Failed to retrieve metadata for ZIP file: {}", e);
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Failed to process uploaded file".to_string(),
                    )
                        .into_response();
                }
            }

            dbg!(&zip_path);
            dbg!(&project_dir);

            // Extract the ZIP archive
            if let Err(e) = extract_zip(&zip_path, &project_dir, &*function_name).await {
                eprintln!("Failed to extract ZIP archive: {}", e);
                return (
                    StatusCode::BAD_REQUEST,
                    format!("Failed to extract ZIP: {}", e),
                )
                    .into_response();
            }

            // TODO check for zip bomb
            // check file size
            // fs::read_dir().await

            // Optionally, remove the ZIP file after extraction
            if let Err(e) = fs::remove_file(&zip_path).await {
                eprintln!("Failed to remove ZIP archive: {}", e);
                // Not critical, so we can continue
            }
        }
        Err(_) => {}
    }

    if let Err(e) = lint_project(&project_dir).await {
        eprintln!("Failed to lint project: {}", e);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Your code contains illegal symbols: ".to_owned() + &e.to_string(),
        )
            .into_response();
    }

    // Build the project
    if let Err(e) = build_project(&project_dir).await {
        eprintln!("Failed to build project: {}", e);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to build project: ".to_owned() + &e.to_string(),
        )
            .into_response();
    }

    let extension = if cfg!(target_os = "linux") {
        "so"
    } else if cfg!(target_os = "macos") {
        "dylib"
    } else if cfg!(target_os = "windows") {
        "dll"
    } else {
        "so"
    };

    let target_release =
        project_dir.join(format!("../target/release/lib{function_name}.{extension}"));
    // stat
    let library_path = fs::metadata(&target_release)
        .await
        .map(|_| target_release)
        .ok();

    let Some(lib_path) = library_path else {
        // eprintln!("No library found in release directory: {}", target_release);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to locate compiled library",
        )
            .into_response();
    };

    let final_libs_dir = "./functions";
    if let Err(e) = fs::create_dir_all(&final_libs_dir).await {
        eprintln!("Failed to create final libs directory: {}", e);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to create final libs directory",
        )
            .into_response();
    }

    let final_lib_path = PathBuf::from(&final_libs_dir).join(function_name.clone());

    if let Err(e) = fs::copy(&lib_path, &final_lib_path).await {
        eprintln!("Failed to copy library to final location: {}", e);
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to store compiled library",
        )
            .into_response();
    }

    println!(
        "Library successfully built and stored at {:?}",
        final_lib_path
    );

    // Flush the cache
    LIB_CACHE.remove(&function_name);

    (
        StatusCode::OK,
        "Cargo project uploaded and built successfully",
    )
        .into_response()
}

async fn extract_zip(
    zip_path: &StdPath,
    extract_to: &StdPath,
    function_name: &str,
) -> Result<(), anyhow::Error> {
    // Read the ZIP file into memory
    let data = fs::read(zip_path).await?;

    // Spawn a blocking task to handle ZIP extraction
    let function_name = function_name.to_string();
    let extract_to = extract_to.to_path_buf();
    let result = tokio::task::spawn_blocking(move || {
        let cursor = io::Cursor::new(data);
        let mut archive = ZipArchive::new(cursor)?;

        for i in 0..archive.len() {
            let mut file = archive.by_index(i)?;

            let outpath = match file.enclosed_name() {
                Some(path) => extract_to.join(path),
                None => continue, // Skip paths that attempt directory traversal
            };

            if (*file.name()).ends_with('/') {
                // If it's a directory, create it
                std::fs::create_dir_all(&outpath)?;
            } else {
                // Ensure parent directories exist
                if let Some(parent) = outpath.parent() {
                    std::fs::create_dir_all(parent)?;
                }

                if file.name() == "build.rs" {
                    println!("Skipping build.rs, compile time code is forbidden.");
                    continue;
                }

                // Skip files inside `target/` directory
                if outpath.starts_with(extract_to.join("target")) {
                    // println!("Skipping file in target directory: {:?}", outpath);
                    continue;
                }

                if file.name() == "Cargo.toml" {
                    // analyze_cargo_file(&outpath.to_str().unwrap())?;
                }

                // Extract the file and handle `.rs` files
                let mut outfile = std::fs::File::create(&outpath)?;
                if file.name().ends_with(".rs") {
                    // analyze_rust_file(&outpath.to_str().unwrap())?;
                }
                io::copy(&mut file, &mut outfile)?;
            }
        }

        Ok::<(), anyhow::Error>(())
    })
    .await??;

    Ok(result)
}

/// Streams the given multipart field to disk at `file_path`.
async fn save_field_to_file(
    path: &PathBuf,
    field: axum::extract::multipart::Field<'_>,
) -> Result<(), io::Error> {
    // Convert the field into an AsyncRead
    let stream = field.map_err(|e| io::Error::new(io::ErrorKind::Other, e));
    let mut stream_reader = StreamReader::new(stream);

    // Create the file and write the bytes from the stream
    let file = File::create(path).await?;
    let mut writer = BufWriter::new(file);
    tokio::io::copy(&mut stream_reader, &mut writer).await?;

    Ok(())
}

/// Simple path validation that ensures the path consists of exactly one "normal" component.
/// This prevents directory traversal attempts like `../../secret`.
fn path_is_valid(path: &str) -> bool {
    let path = Path::new(path);
    let mut components = path.components().peekable();

    if let Some(first) = components.peek() {
        // If the first component is not normal, reject.
        // i.e., no leading slashes, no references to current/parent dirs, etc.
        if !matches!(first, std::path::Component::Normal(_)) {
            return false;
        }
    }

    // Ensure there's exactly 1 component total.
    components.count() == 1
}

pub fn generate_hmac(data: &str, secret: &str) -> String {
    let mut key = [0u8; 32];
    let secret_bytes = secret.as_bytes();

    // Copy up to 32 bytes from the secret into the key
    let len = secret_bytes.len().min(32);
    key[..len].copy_from_slice(&secret_bytes[..len]);

    blake3::keyed_hash(
        &key,
        data.as_bytes(),
    ).to_string()
}


// pub fn generate_hmac(data: &str, secret: &str) -> String {
//     return "function".to_string();
//     type HmacSha256 = Hmac<Sha256>;
//
//     let mut mac = HmacSha256::new_from_slice(secret.as_ref())
//         .expect("HMAC can take key of any size");
//     mac.update(data.as_ref());
//
//     // `result` has type `CtOutput` which is a thin wrapper around array of
//     // bytes for providing constant time equality check
//     let result = mac.finalize();
//     // To get underlying array use `into_bytes`, but be careful, since
//     // incorrect use of the code value may permit timing attacks which defeats
//     // the security provided by the `CtOutput`
//     let code_bytes = result.into_bytes();
//     hex::encode(code_bytes)
// }