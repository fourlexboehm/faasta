use crate::NewArgs;
use std::error::Error;
use std::path::Path;
use std::{env, fs, io};

pub const HTTP_CARGO_TOML: &str = include_str!("../../function/Cargo.toml");
pub const HTTP_LIB_RS: &str = include_str!("../../function/src/lib.rs");
pub fn handle_new(args: &NewArgs) -> Result<(), Box<dyn Error>> {
    dbg!(&args);
    let current_dir = env::current_dir()?;
    let new_project_dir = current_dir.join(&args.package_name);

    if new_project_dir.exists() && args.package_name != "" {
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
    let pkg_name = if args.package_name == "" {
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


/// Writes the embedded Cargo.toml & main.rs to disk for event packages,
/// updating the `[package] name` in Cargo.toml to `package_name` and
/// inserting the selected `event_type` into main.rs.
fn write_event_files(
    project_dir: &Path,
    cargo_toml_str: &str,
    main_rs_str: &str,
    package_name: &str,
    event_type: &str,
) -> io::Result<()> {
    let cargo_toml_path = project_dir.join("Cargo.toml");
    let updated_cargo_toml = rewrite_package_name(cargo_toml_str, package_name);
    fs::write(cargo_toml_path, updated_cargo_toml)?;

    // Find the position after the last "::"
    let tail_start = event_type.rfind("::").unwrap() + 2;

    // Extract the substring after "::"
    let tail_substring = &event_type[tail_start..];

    // TODO: This wont work for all event types, figure out a more robust way, either using reflection or a map
    // Replace "Event" with "Data" in that tail substring
    let new_tail = tail_substring.replace("Event", "Data");

    // Build the final string for main.rs
    let updated_main_rs = main_rs_str
        .replace(
            "google_cloudevents::google::events::cloud::pubsub::v1::MessagePublishedData",
            event_type,
        )
        // Now swap out just the tail
        .replace("MessagePublishedData", &new_tail);

    // Write changes to src/main.rs
    let main_rs_path = project_dir.join("src").join("main.rs");
    fs::write(main_rs_path, updated_main_rs)?;
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

    // 2. Write src/main.rs
    let main_rs_path = project_dir.join("src").join("main.rs");
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
            output.push_str(&format!("name = \"{}\"\n", package_name));
            in_package = false; // Only replace once
            continue;
        }

        output.push_str(line);
        output.push('\n');
    }

    output
}
