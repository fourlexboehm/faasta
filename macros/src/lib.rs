use proc_macro::TokenStream;
use quote::quote;
use std::env;
use syn::{parse_macro_input, ItemFn};

/// Transforms an `async fn` into an exported function suitable for use in FaaS-based runtimes.
///
/// # Overview
///
/// The `#[faasta]` attribute takes an async function with certain parameters and generates a
/// new exported function named `dy_<HMAC>`, where `<HMAC>` is derived from:
///
/// 1. The crate's package name, obtained from the `CARGO_PKG_NAME` environment variable (or
///    defaults to `default_package` if not set).
/// 2. A secret key from the `FAASTA_HMAC_SECRET` environment variable (or defaults to
///    `default_secret` if not set).
///
/// The generated function has an ABI of `extern "Rust"` and returns a
/// `Pin<Box<dyn Future<Output = Response<Body>> + Send + 'static>>`. This allows it to be
/// consumed by the faasta function-as-a-service (FaaS) runtimes while preserving the async execution model.
///
/// # Usage
///
/// 1. Mark your async function with the `#[faasta]` attribute.
/// 2. Ensure your function arguments are only among the supported types:
///    - [`Method`](https://docs.rs/http/latest/http/struct.Method.html)
///    - [`Uri`](https://docs.rs/http/latest/http/uri/struct.Uri.html)
///    - [`HeaderMap`](https://docs.rs/http/latest/http/header/struct.HeaderMap.html)
///    - [`Bytes`](https://docs.rs/bytes/latest/bytes/struct.Bytes.html)
///    - `Dir` (a custom type provided by your application)
/// 3. Return a [`Response<Body>`](https://docs.rs/http/latest/http/response/struct.Response.html).
///
/// ```rust
/// # use axum::Bytes;
/// # use axum::{Method, StatusCode};
/// # use axum::response::Response;
/// # use axum::Body;
/// # use faasta_macro::faasta;
/// # use CapStd::fs::Dir;
///
/// #[faasta]
/// async fn handler(method: Method, body: Bytes, dir: Dir) -> Response<Body> {
///     Response::builder()
///         .status(StatusCode::OK)
///         .body(Body::from("HELLO WORLD"))
///         .unwrap()
/// }
/// ```
///
/// The macro will generate code similar to:
///
/// ```ignore
/// #[no_mangle]
/// pub extern "Rust" fn dy_<HMAC>(
///     method: Method,
///     uri: Uri,
///     headers: HeaderMap,
///     body: Bytes,
///     dir: Dir
/// ) -> Pin<Box<dyn Future<Output = Response<Body>> + Send + 'static>> {
///     Box::pin(async move {
///         // Original function body
///     })
/// }
/// ```
///
/// # Compile Errors
///
/// A compile error will occur if:
///
/// - The function is not `async`.
/// - Any function parameter type is not among the supported types (listed above).
///
/// # Environment Variables
///
/// - `CARGO_PKG_NAME`: Used to determine the crate name. Defaults to `default_package` if not set.
/// - `FAASTA_HMAC_SECRET`: Used for computing the HMAC-based suffix. Defaults to `default_secret` if not set.
///
/// # Example
///
/// ```rust,ignore
/// // In Cargo.toml, name might be "my-awesome-service"
/// // $ export CARGO_PKG_NAME="my-awesome-service"
/// // $ export FAASTA_HMAC_SECRET="top_secret"
///
/// #[faasta]
/// async fn my_handler(method: Method, body: Bytes, dir: Dir) -> Response<Body> {
///     Response::builder()
///         .status(StatusCode::OK)
///         .body(Body::from("Hello from my-awesome-service!"))
///         .unwrap()
/// }
/// // This compiles into a function named something like `dy_abcd1234(...)`.
/// ```
#[proc_macro_attribute]
pub fn faasta(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemFn);
    let vis = input.vis;
    let sig = input.sig;
    let fn_args = &sig.inputs;
    let block = input.block;

    let supported_args = ["Method", "Uri", "HeaderMap", "Bytes", "Dir"];

    // Validate function arguments
    for arg in fn_args.iter() {
        if let syn::FnArg::Typed(pat_type) = arg {
            if let syn::Type::Path(type_path) = &*pat_type.ty {
                let arg_type = type_path.path.segments.last().unwrap().ident.to_string();
                if !supported_args.contains(&arg_type.as_str()) {
                    return syn::Error::new_spanned(
                        pat_type,
                        format!(
                            "Unsupported argument type: {}. Supported types are: {:?}",
                            arg_type, supported_args
                        ),
                    )
                    .to_compile_error()
                    .into();
                }
            }
        }
    }

    // Get the package name from the environment
    let package_name = env::var("CARGO_PKG_NAME").unwrap_or_else(|_| "default_package".to_string());

    // Generate HMAC-based hash for function renaming
    let secret_key =
        env::var("FAASTA_HMAC_SECRET").unwrap_or_else(|_| "default_secret".to_string());
    let hashed_suffix = generate_hmac(&package_name, &secret_key);

    // Generate the new function name
    let new_fn_name = quote::format_ident!("dy_{}", hashed_suffix);
    println!("Generated function name: {}", new_fn_name);

    // Generate the wrapper function
    let output = quote! {
        #[no_mangle]
        #vis extern "Rust" fn #new_fn_name(
            method: Method,
            uri: Uri,
            headers: HeaderMap,
            body: Bytes,
            dir: Dir
        ) -> Pin<Box<dyn Future<Output = Response<Body>> + Send + 'static>> {
            Box::pin(async move {
                #block
            })
        }
    };

    output.into()
}
// Helper function to generate HMAC
// this  function is duplicated in the server so the macro doesn't require any dependencies
fn generate_hmac(data: &str, secret: &str) -> String {
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