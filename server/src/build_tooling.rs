use crate::LIB_CACHE;
use analyze::{analyze_cargo_file, analyze_rust_file};
use axum::extract::Path as AxumPath;
use axum::{extract::Multipart, http::StatusCode, response::IntoResponse};
use bytes::Bytes;
use futures::{Stream, TryFutureExt, TryStreamExt};
use geiger::IncludeTests;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::io::Read;
use std::path::Path as StdPath;
use std::{
    io,
    path::{Path, PathBuf},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::{
    fs::{self, File},
    io::BufWriter,
    process::Command,
};
use tokio_util::io::StreamReader;
use uuid::Uuid;
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

    // Create a unique project directory based on the function name
    let project_dir = PathBuf::from(BUILDS_DIRECTORY).join(&function_name);
    let hmac = generate_hmac(&function_name);

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

            // Optional: Validate the uploaded file's filename or MIME type
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

    // rename_entrypoint(&project_dir, &function_name)
    //     .await
    //     .unwrap_or_else(|e| {
    //         eprintln!("Failed to rename entrypointin project: {}", e);
    //         (
    //             StatusCode::INTERNAL_SERVER_ERROR,
    //             "Failed to sanitize project".to_owned(),
    //         )
    //             .into_response();
    //     });
    //
    build_project(&project_dir).await.unwrap_or_else(|e| {
        eprintln!("Failed to build project: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to build project".to_owned(),
        )
            .into_response();
    });

    // Identify the compiled library
    // Normally, the library name depends on the Cargo package name and platform.
    // For demonstration, we assume it is named `libmycrate.so` (Linux),
    // or we search the target folder for any `.so`/`.dll`/`.dylib`.
    // Adjust to your real scenario.

    let target_release = project_dir.join("../target/release/");

    // Let's do a simple approach: look for any file that starts with `lib`
    // and ends with `.so`, `.dll`, or `.dylib`.
    let mut library_path = None;
    if let Ok(mut entries) = fs::read_dir(&target_release).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let file_path = entry.path();
            if let Some(file_name) = file_path.file_name().and_then(|s| s.to_str()) {
                let is_shared_obj = file_name.starts_with("lib")
                    && (file_name.ends_with(".so")
                        || file_name.ends_with(".dll")
                        || file_name.ends_with(".dylib"));
                if is_shared_obj {
                    library_path = Some(file_path);
                    break;
                }
            }
        }
    }

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

// async fn rename_entrypoint(
//     project_dir: &PathBuf,
//     function_name: &str,
// ) -> Result<(), anyhow::Error> {
//     let mut lib = File::open(StdPath::new(project_dir).join("src/lib.rs")).await?;
//     let mut lib_string = String::new();
//     lib.read_to_string(&mut lib_string).await?;
//
//     let updated_lib_string = lib_string.replace(
//         "handler_dy(",
//         &format!("{hmac}(", hmac = generate_hmac(function_name)),
//     );
//     println!("{}", updated_lib_string);
//     lib.try_clone().await?;
//     let mut lib = File::open(StdPath::new(project_dir).join("src/lib.rs")).await?;
//     lib.write_all(updated_lib_string.as_bytes()).await?;
//     Ok(())
// }
async fn extract_zip(zip_path: &StdPath, extract_to: &StdPath, function_name: &str) -> Result<(), anyhow::Error> {
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
                    println!("Skipping build.rs");
                    continue;
                }

                // Skip files inside `target/` directory
                if outpath.starts_with(extract_to.join("target")) {
                    println!("Skipping file in target directory: {:?}", outpath);
                    continue;
                }

                if file.name() == "Cargo.toml" {
                    // analyze_cargo_file(&outpath.to_str().unwrap())?;
                }

                // Extract the file and handle `.rs` files
                let mut outfile = std::fs::File::create(&outpath)?;
                if file.name().ends_with(".rs") {
                    // analyze_rust_file(&outpath.to_str().unwrap())?;
                    let mut contents = String::new();
                    file.read_to_string(&mut contents);

                    // Perform the replacement
                    let updated_contents = contents.replace(
                        "handler_dy(",
                        &format!("dy_{hmac}(", hmac = generate_hmac(&*function_name)),
                    );

                    // Write the updated contents back to the output path
                    io::Write::write_all(&mut outfile, updated_contents.as_bytes())?;
                } else {
                    io::copy(&mut file, &mut outfile)?;
                }
            }
        }

        Ok::<(), anyhow::Error>(())
    })
    .await??;

    Ok(result)
}

/// Runs `cargo build --release` in the specified project directory.
async fn build_project(project_dir: &StdPath) -> Result<(), anyhow::Error> {
    let output = Command::new("cargo")
        .arg("build")
        .arg("--release")
        .arg("--target-dir")
        .arg("../target")
        .current_dir(project_dir)
        .output()
        .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("Cargo build failed: {}", stderr));
    }

    Ok(())
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

pub fn generate_hmac(data: &str) -> String {
    type HmacSha256 = Hmac<Sha256>;

    let mut mac = HmacSha256::new_from_slice(b"HAEC*#BGE*QJOTBbhodegB@8BHAcH")
        .expect("HMAC can take key of any size");
    mac.update(data.as_ref());

    // `result` has type `CtOutput` which is a thin wrapper around array of
    // bytes for providing constant time equality check
    let result = mac.finalize();
    // To get underlying array use `into_bytes`, but be careful, since
    // incorrect use of the code value may permit timing attacks which defeats
    // the security provided by the `CtOutput`
    let code_bytes = result.into_bytes();
    hex::encode(code_bytes)
}
