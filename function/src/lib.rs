use std::future::Future;
use std::pin::Pin;
use axum::body::{Body, Bytes};
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::Response;
use cap_async_std::fs::Dir;
use faas_proc_macros::faasta;
// #[no_mangle]
// pub extern "Rust" fn handler_dy(
//     method: Method,
//     uri: Uri,
//     headers: HeaderMap,
//     body: Bytes,
// ) -> Pin<Box<dyn Future<Output = Response<Body>> + Send + 'static>> {
//     Box::pin(async move {
//         handler(method, uri, headers, body).await
//     })
//
//
// }

// #[faasta]
// async fn handler(body: Bytes, dir: Dir) -> Response<Body> {
//     dir.
//     Response::new(Body::from("Hello World"))
// }

// #[faasta]
// async fn handler(body: Bytes, dir: Dir) -> Response<Body> {
//
//     // Open the database file from the directory
//     let db_path = dir.open("data.sqlite").await.unwrap();
//
//     // Convert to a URI for sqlx
//     let db_uri = format!("sqlite://{}", db_path.canonicalize().unwrap().display());
//
//     // Create a connection pool
//     let pool = SqlitePoolOptions::new()
//         .max_connections(1)
//         .connect(&db_uri)
//         .await
//         .map_err(|_| {
//             Response::builder()
//                 .status(500)
//                 .body(Body::from("Failed to connect to database"))
//                 .unwrap()
//         })?;
//
//     // Query a simple result from the `strings` table
//     let result = sqlx::query("SELECT value FROM strings WHERE key = ?")
//         .bind("Hello")
//         .fetch_one(&pool)
//         .await
//         .map_err(|_| {
//             Response::builder()
//                 .status(500)
//                 .body(Body::from("Failed to query database"))
//                 .unwrap()
//         })?;
//
//     // Extract the value and return it
//     let value: String = result.try_get("value").unwrap_or_else(|_| "world".to_string());
//     let response_body = format!("Hello -> {}", value);
//
//     Response::builder()
//         .status(200)
//         .body(Body::from(response_body))
//         .unwrap()
// }
// async fn handler(method: Method, uri: Uri, headers: HeaderMap, body: Bytes) -> Response<Body> {
#[faasta]
async fn handler(method: Method, uri: Uri, headers: HeaderMap, body: Bytes, dir: Dir) -> Response<Body> {
    let _ = cap_async_std::ambient_authority();
    Response::builder().status(StatusCode::OK).body(Body::from("HELLO WORLD")).unwrap()
}