use axum::body::{Body, Bytes};
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::Response;
use cap_async_std::fs::Dir;
use faasta_macros::faasta;
use std::future::Future;
use std::pin::Pin;

/// This is the main handler function for your FaaSta serverless application.
/// 
/// The #[faasta] macro is required and makes this function the entry point.
/// The function signature must match exactly as shown below.
///
/// Parameters:
/// - method: HTTP method of the request (GET, POST, etc.)
/// - uri: URI of the request 
/// - headers: HTTP headers from the request
/// - body: Request body as bytes
/// - dir: A capability-based directory handle for file operations
#[faasta]
async fn handler(method: Method, uri: Uri, headers: HeaderMap, body: Bytes, dir: Dir) -> Response<Body> {
    // You can access different parts of the request:
    let path = uri.path();
    let method_str = method.as_str();

    // Example of using the path to determine response
    if path.ends_with("/hello") {
        return Response::builder()
            .status(StatusCode::OK)
            .body(Body::from("Hello, World!"))
            .unwrap();
    }
    
    // Default response
    Response::builder()
        .status(StatusCode::OK)
        .body(Body::from(format!("FaaSta function received {} request to {}", method_str, path)))
        .unwrap()
}
