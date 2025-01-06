use std::future::Future;
use std::pin::Pin;
use axum::body::{Body, Bytes};
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::Response;

#[no_mangle]
pub extern "Rust" fn handler_dy(
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Pin<Box<dyn Future<Output = Response<Body>> + Send + 'static>> {
    Box::pin(async move {
        handler(method, uri, headers, body).await
    })


}

async fn handler(method: Method, uri: Uri, headers: HeaderMap, body: Bytes) -> Response<Body> {
    Response::builder().status(StatusCode::OK).body(Body::from("HELLO WORLD")).unwrap()
}