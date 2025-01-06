mod build_tooling;

use crate::build_tooling::{generate_hmac, handle_upload_and_build};
use axum::body::Body;
use axum::error_handling::HandleErrorLayer;
use axum::extract::Path;
use axum::response::Response;
use axum::{
    body::Bytes,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    BoxError, Router,
};
use chashmap::CHashMap;
use http::{HeaderMap, Method, Uri};
use lazy_static::lazy_static;
use libloading::{Library, Symbol};
use std::error::Error;
use std::fmt;
use std::fmt::{Display, Formatter};
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::AtomicUsize;
use std::time::Duration;
use tokio::fs;
use tower::timeout::TimeoutLayer;
use tower::{timeout, ServiceBuilder};
use tower_http::catch_panic::CatchPanicLayer;


// type HandleRequestFn = fn(Method, Uri, HeaderMap, Bytes) -> Response;
type HandleRequestFn =
    extern "Rust" fn(
        Method,
        Uri,
        HeaderMap,
        Bytes,
    ) -> Pin<Box<dyn Future<Output = Response<Body>> + Send + 'static>>;

lazy_static! {
    /// This is an example for using doc comment attributes
    static ref LIB_CACHE: CHashMap<String, (Library, AtomicUsize)> = CHashMap::new();
}

async fn handle_invoke_rs(
    Path(function_name): Path<String>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    // dbg!(&function_name);
    let start_time = std::time::Instant::now();
    let lib = match LIB_CACHE.get(&function_name) {
        Some(lib) => {
            lib
        }
        None => {
            // Otherwise, open it
            let path = format!("./functions/{name}", name = function_name);
            if fs::try_exists(&path).await.is_err() {
                return (StatusCode::NOT_FOUND, "Function not found").into_response();
            }
            let new_lib = unsafe {
                match Library::new(path) {
                    Ok(lib) => lib,
                    Err(_) => {
                        return (StatusCode::NOT_FOUND, "Function could not be loaded")
                            .into_response()
                    }
                }
            };
            // Insert into our map
            LIB_CACHE.insert(function_name.clone(), (new_lib, AtomicUsize::new(1)));
            // if LIB_CACHE.len() > 1000 {
            //     tokio::spawn(async move {
            //         // Remove the least used function
            //         LIB_CACHE.iter().min_by_key(|(_, v)| v.1.load(std::sync::atomic::Ordering::Relaxed)).map(|(least_used, _)| {
            //             LIB_CACHE.remove(&least_used);
            //         })
            //     });
            // }
            LIB_CACHE.get(&function_name).unwrap()
        }
    };
    lib.1.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let hmac = "dy_".to_string() + &*generate_hmac(&*function_name);
    println!("{}", hmac);
    let handle_request: Symbol<HandleRequestFn> = unsafe { lib.0.get(hmac.as_ref()).unwrap() };

    let ah = handle_request(method, uri, headers, body).await;
    println!("Function {} took {:?}", function_name, start_time.elapsed());
    ah
}
#[derive(Debug)]
struct TimeoutError {
    message: String,
}

impl TimeoutError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl Display for TimeoutError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl Error for TimeoutError {}

impl IntoResponse for TimeoutError {
    fn into_response(self) -> Response {
        (StatusCode::GATEWAY_TIMEOUT, self.message).into_response()
    }
}

// This is our handler for timeouts (or other errors) produced by `TimeoutLayer`
async fn handle_timeout_error(error: BoxError) -> TimeoutError {
    if error.is::<timeout::error::Elapsed>() {
        TimeoutError::new("Request timed out")
    } else {
        TimeoutError::new(format!("Unhandled error: {}", error))
    }
}
#[tokio::main]
async fn main() {
    let service = ServiceBuilder::new()
        .layer(CatchPanicLayer::new())
        .layer(HandleErrorLayer::new(handle_timeout_error))
        .layer(TimeoutLayer::new(Duration::from_secs(900)));
    let app = Router::new()
        // POST /upload
        .route("/upload/{function_name}", post(handle_upload_and_build))
        // .route("/upload", post(handle_upload))
        // GET /invoke/{function_name}
        .route(
            "/{function_name}",
            get(handle_invoke_rs).post(handle_invoke_rs),
        )
        .layer(service);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await.unwrap();
    axum::serve(listener, app.into_make_service())
        .await
        .unwrap();
}

//
// pub async fn handle_timeout_error(err: BoxError) -> AppError {
//     if err.is::<timeout::error::Elapsed>() {
//         AppError::RequestTimeout
//     } else {
//         AppError::Unexpected(err)
//     }
// }
// type HandleRequestCFn = unsafe extern "C" fn(*const RequestInfo) -> *mut ResponseInfo;
//
// async fn handle_invoke_c_compat(
//     Path(function_name): Path<String>,
//     method: Method,
//     uri: Uri,
//     headers: HeaderMap,
//     body: Bytes,
// ) -> impl IntoResponse {
//     // 1. Read the entire request body
//     // 2. Convert strings to CStrings
//     let method_c = CString::new(method.to_string()).unwrap();
//     let uri_string = uri.to_string();
//     let func_name = match uri_string.split_once("/") {
//         None => uri_string,
//         Some(it) => it.0.to_string()
//     };
//     let uri_c = CString::new(uri.to_string()).unwrap();
//     let path_c = CString::new(uri.path().to_string()).unwrap();
//     let query_c = uri
//         .query()
//         .map(|q| CString::new(q).unwrap())
//         .unwrap_or_else(|| CString::new("").unwrap());
//
//     // 3. Build an array of KeyValuePair for headers
//     let mut kv_pairs: Vec<KeyValuePair> = Vec::with_capacity(headers.len());
//     let mut cstrings_holder = Vec::new(); // to hold the memory for the CStrings
//     for (name, value) in headers.iter() {
//         let key_c = CString::new(name.as_str()).unwrap();
//         let val_c = CString::new(value.to_str().unwrap()).unwrap();
//         kv_pairs.push(KeyValuePair {
//             key: key_c.as_ptr(),
//             value: val_c.as_ptr(),
//         });
//         // We must keep these CStrings alive until after the call
//         cstrings_holder.push(key_c);
//         cstrings_holder.push(val_c);
//     }
//
//     // 4. Construct RequestInfo
//     let req_info = RequestInfo {
//         method: method_c.as_ptr(),
//         uri: uri_c.as_ptr(),
//         path: path_c.as_ptr(),
//         query: query_c.as_ptr(),
//         headers: kv_pairs.as_ptr(),
//         headers_len: kv_pairs.len(),
//         body: body.as_ptr(),
//         body_len: body.len(),
//     };
//
//     // 5. Load library and symbol
//     // TODO USE .SO on linux
//     let lib = unsafe { Library::new("uploads/".to_string() + &*function_name + ".dylib") }.unwrap();
//     let handle_request: Symbol<HandleRequestCFn> = unsafe { lib.get(b"handle_request") }.unwrap();
//
//     // 6. Call the function
//     let resp_ptr = unsafe { handle_request(&req_info as *const RequestInfo) };
//     if resp_ptr.is_null() {
//         return (StatusCode::INTERNAL_SERVER_ERROR, "null response").into_response();
//     }
//
//     // 7. Convert *mut ResponseInfo back to a Box, so we can safely access and eventually free it
//     let resp_box = unsafe { Box::from_raw(resp_ptr) };
//
//     // 8. Extract data
//     let status_code = resp_box.status_code;
//     let body_ptr = resp_box.body;
//     let _body_len = resp_box.body_len; // if you set this, you can read raw bytes
//
//     // 9. Convert the returned body pointer to a String
//     let body_str = if !body_ptr.is_null() {
//         let c_slice = unsafe { CStr::from_ptr(body_ptr) };
//         let s = c_slice.to_string_lossy().to_string();
//         // The library allocated this string, so we should free it if it used `CString::into_raw()`
//         // But we have no direct pointer to free unless we define a separate `free_string` export
//         // or we embedded the logic in handle_request.
//         s
//     } else {
//         "".to_string()
//     };
// // 10. Return the response
// let sc = StatusCode::from_u16(status_code).unwrap_or(StatusCode::OK);
// (sc, body_str).into_response()
// }
