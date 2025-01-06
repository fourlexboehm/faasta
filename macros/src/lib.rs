use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, ItemFn};
use std::env;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use hex;
use hmac::KeyInit;

/// faasta
/// usage:
/// #[faasta]
/// async fn handler(method: Method, body: Bytes, dir: Dir) -> Response<Body> {
///     Response::builder().status(StatusCode::OK).body(Body::from("HELLO WORLD")).unwrap()
/// }
#[proc_macro_attribute]
pub fn faasta(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as ItemFn);
    let vis = input.vis;
    let sig = input.sig;
    let fn_name = &sig.ident;
    let fn_args = &sig.inputs;
    let block = input.block;

    let supported_args = [
        "Method",
        "Uri",
        "HeaderMap",
        "Bytes",
        "Dir",
    ];

    // Validate function arguments
    for arg in fn_args.iter() {
        if let syn::FnArg::Typed(pat_type) = arg {
            if let syn::Type::Path(type_path) = &*pat_type.ty {
                let arg_type = type_path.path.segments.last().unwrap().ident.to_string();
                if !supported_args.contains(&arg_type.as_str()) {
                    return syn::Error::new_spanned(
                        pat_type,
                        format!("Unsupported argument type: {}. Supported types are: {:?}", arg_type, supported_args),
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
    let secret_key = env::var("FAASTA_HMAC_SECRET").unwrap_or_else(|_| "default_secret".to_string());
    let hashed_suffix = generate_hmac(&package_name, &secret_key);

    // Generate the new function name
    let new_fn_name = quote::format_ident!("dy_{}", hashed_suffix);

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
}// Helper function to generate HMAC
fn generate_hmac(data: &str, secret: &str) -> String {
    type HmacSha256 = Hmac<Sha256>;

    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .expect("HMAC can take key of any size");
    mac.update(data.as_ref());

    let result = mac.finalize();
    let code_bytes = result.into_bytes();
    hex::encode(code_bytes)
}