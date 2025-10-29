#![warn(unused_extern_crates)]

use anyhow::{Context, Result};
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::{Host, OriginalUri, Path, State};
use axum::http::{HeaderMap, Request, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum_server::tls_rustls::RustlsConfig;
use clap::Parser;
use faasta_interface::FunctionError;
use serde::Serialize;
use serde_json::json;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::TcpListener;
use tower::ServiceBuilder;
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::trace::TraceLayer;
use tracing::{Level, error, info};

mod cert_manager;
mod github_auth;
mod kvm_guest;
mod metrics;
mod quic;
mod rpc_service;
mod wasi_server;

use cert_manager::CertManager;
use metrics::{get_metrics, spawn_periodic_flush};
use rpc_service::create_service;
use wasi_server::{FaastaServer, SERVER, sanitize_function_name};

#[derive(Parser, Debug, Clone)]
#[command(name = "server")]
#[command(about = "Faasta KVM HTTP Function Server", long_about = None)]
struct Args {
    /// Address to listen on (e.g., 0.0.0.0:443)
    #[arg(short, long, env = "LISTEN_ADDR", default_value = "0.0.0.0:443")]
    listen_addr: SocketAddr,

    /// HTTP Address to listen on for redirects (e.g., 0.0.0.0:80)
    #[arg(long, env = "HTTP_LISTEN_ADDR", default_value = "0.0.0.0:80")]
    http_listen_addr: SocketAddr,

    /// Base domain for function subdomains
    #[arg(long, env = "BASE_DOMAIN", default_value = "faasta.xyz")]
    base_domain: String,

    /// Path to the TLS certificate file (PEM format)
    #[arg(long, env = "TLS_CERT", default_value = "./certs/cert.pem")]
    tls_cert_path: PathBuf,

    /// Path to the TLS private key file (PEM format)
    #[arg(long, env = "TLS_KEY", default_value = "./certs/key.pem")]
    tls_key_path: PathBuf,

    /// Path to the certs directory
    #[arg(long, env = "CERTS_DIR", default_value = "./certs")]
    certs_dir: PathBuf,

    /// Path to the SledDB database directory
    #[arg(long, env = "DB_PATH", default_value = "./data/db")]
    db_path: PathBuf,

    /// Path to the functions directory containing uploaded shared objects
    #[arg(long, env = "FUNCTIONS_PATH", default_value = "./functions")]
    functions_path: PathBuf,

    /// Address for the RPC server (QUIC)
    #[arg(long, env = "RPC_LISTEN_ADDR", default_value = "0.0.0.0:2443")]
    rpc_listen_addr: String,

    /// Auto-generate TLS certificate using Porkbun
    #[arg(long, env = "AUTO_CERT", default_value = "false")]
    auto_cert: bool,
}

#[derive(Clone)]
struct AppState {
    server: Arc<FaastaServer>,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install rustls crypto provider");

    let _ = dotenvy::dotenv();
    tracing_subscriber::fmt().with_max_level(Level::INFO).init();

    let args = Args::parse();

    std::fs::create_dir_all(&args.db_path)
        .with_context(|| format!("failed to create db directory at {:?}", args.db_path))?;
    std::fs::create_dir_all(&args.functions_path).with_context(|| {
        format!(
            "failed to create functions directory at {:?}",
            args.functions_path
        )
    })?;
    std::fs::create_dir_all(&args.certs_dir)
        .with_context(|| format!("failed to create cert directory at {:?}", args.certs_dir))?;

    if args.auto_cert {
        let cert_manager = Arc::new(CertManager::new(
            args.base_domain.clone(),
            args.certs_dir.clone(),
            args.tls_cert_path.clone(),
            args.tls_key_path.clone(),
        ));
        cert_manager
            .obtain_or_renew_certificate()
            .await
            .context("failed to obtain TLS certificate")?;
        cert_manager.spawn_periodic_renewal();
    }

    let metadata_db = sled::open(&args.db_path).context("failed to open sled db")?;

    let server = Arc::new(
        FaastaServer::new(
            metadata_db,
            args.base_domain.clone(),
            args.functions_path.clone(),
        )
        .await?,
    );
    SERVER
        .set(server.clone())
        .map_err(|_| anyhow::anyhow!("server already initialised"))?;

    spawn_periodic_flush(60);

    let app_state = AppState {
        server: server.clone(),
    };

    let router = Router::new()
        .route("/healthz", get(health_handler))
        .route("/v1/metrics", get(metrics_handler))
        .route("/v1/publish/:function_name", post(publish_handler))
        .fallback(function_dispatch)
        .with_state(app_state)
        .layer(
            ServiceBuilder::new()
                .layer(CatchPanicLayer::new())
                .layer(TraceLayer::new_for_http()),
        );

    let rustls_config =
        RustlsConfig::from_pem_file(args.tls_cert_path.clone(), args.tls_key_path.clone())
            .await
            .context("failed to load tls assets")?;

    let redirect_domain = args.base_domain.clone();
    tokio::spawn(run_http_redirect(args.http_listen_addr, redirect_domain));

    let rpc_cert = args.tls_cert_path.clone();
    let rpc_key = args.tls_key_path.clone();
    let rpc_addr = args.rpc_listen_addr.clone();
    tokio::task::spawn_blocking(move || match compio::runtime::Runtime::new() {
        Ok(runtime) => {
            if let Err(err) = runtime.block_on(quic::run_rpc_server(rpc_cert, rpc_key, rpc_addr)) {
                error!("rpc server exited with error: {err}");
            }
        }
        Err(err) => error!("failed to start compio runtime for rpc server: {err}"),
    });

    info!("HTTPS server listening on {}", args.listen_addr);
    axum_server::bind_rustls(args.listen_addr, rustls_config)
        .serve(router.into_make_service())
        .await
        .context("https server error")
}

async fn run_http_redirect(addr: SocketAddr, target_domain: String) {
    let listener = match TcpListener::bind(addr).await {
        Ok(listener) => listener,
        Err(err) => {
            error!("failed to bind HTTP redirect listener: {err}");
            return;
        }
    };

    let app = Router::new()
        .fallback(redirect_handler)
        .with_state(target_domain.clone());

    if let Err(err) = axum::serve(listener, app.into_make_service()).await {
        error!("http redirect server exited with error: {err}");
    }
}

async fn redirect_handler(
    State(target_domain): State<String>,
    OriginalUri(uri): OriginalUri,
) -> impl IntoResponse {
    let location = format!("https://{}{}", target_domain, uri.path());
    Response::builder()
        .status(StatusCode::MOVED_PERMANENTLY)
        .header(header::LOCATION, location)
        .body(Body::empty())
        .unwrap()
}

async fn health_handler() -> impl IntoResponse {
    Response::builder()
        .status(StatusCode::OK)
        .body(Body::from("ok"))
        .unwrap()
}

async fn metrics_handler() -> impl IntoResponse {
    json_response(StatusCode::OK, get_metrics())
}

async fn publish_handler(
    Path(function_name): Path<String>,
    request: Request<Body>,
) -> impl IntoResponse {
    let Some(sanitized_name) = sanitize_function_name(&function_name) else {
        return error_response(StatusCode::BAD_REQUEST, "Invalid function name");
    };

    let token_header = match request.headers().get(header::AUTHORIZATION) {
        Some(value) => value,
        None => return error_response(StatusCode::UNAUTHORIZED, "Missing Authorization header"),
    };

    let token = match token_header.to_str() {
        Ok(token) => token.trim().trim_start_matches("Bearer ").to_string(),
        Err(_) => return error_response(StatusCode::UNAUTHORIZED, "Invalid Authorization header"),
    };

    let body_bytes = match to_bytes(request.into_body(), usize::MAX).await {
        Ok(bytes) => bytes,
        Err(err) => {
            error!("failed to read publish body: {err}");
            return error_response(StatusCode::BAD_REQUEST, "Failed to read request body");
        }
    };

    if body_bytes.is_empty() {
        return error_response(StatusCode::BAD_REQUEST, "Empty artifact body");
    }

    let service = match create_service() {
        Ok(service) => service,
        Err(err) => {
            error!("failed to create publish service: {err}");
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    match service
        .publish_impl(body_bytes.to_vec(), sanitized_name.clone(), token)
        .await
    {
        Ok(message) => json_response(
            StatusCode::OK,
            json!({
                "success": true,
                "message": message,
            }),
        ),
        Err(err) => {
            let status = map_function_error(&err);
            json_response(
                status,
                json!({
                    "success": false,
                    "error": err.to_string(),
                }),
            )
        }
    }
}

async fn function_dispatch(
    State(state): State<AppState>,
    host: Option<Host>,
    request: Request<Body>,
) -> impl IntoResponse {
    let host_string = host.map(|Host(host)| host);
    let host_ref = host_string.as_deref();
    let method = request.method().clone();
    let uri = request.uri().clone();
    let headers: HeaderMap = request.headers().clone();

    let body_bytes = match to_bytes(request.into_body(), usize::MAX).await {
        Ok(bytes) => bytes,
        Err(err) => {
            error!("failed to read request body: {err}");
            return error_response(StatusCode::BAD_REQUEST, "Failed to read request body");
        }
    };

    let Some(function_name) =
        wasi_server::resolve_function_name(host_ref, uri.path(), &state.server.base_domain)
    else {
        return error_response(StatusCode::NOT_FOUND, "Function name missing");
    };

    let Some(sanitized_function) = sanitize_function_name(&function_name) else {
        return error_response(StatusCode::BAD_REQUEST, "Invalid function name");
    };

    if !state.server.function_exists(&sanitized_function) {
        return error_response(StatusCode::NOT_FOUND, "Function not found");
    }

    match state
        .server
        .invoke(&sanitized_function, method, uri, headers, body_bytes)
        .await
    {
        Ok(response) => response,
        Err(err) => {
            error!("function invocation failed: {err}");
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Function invocation failed",
            )
        }
    }
}

fn map_function_error(error: &FunctionError) -> StatusCode {
    match error {
        FunctionError::AuthError(_) => StatusCode::UNAUTHORIZED,
        FunctionError::NotFound(_) => StatusCode::NOT_FOUND,
        FunctionError::PermissionDenied(_) => StatusCode::FORBIDDEN,
        FunctionError::InvalidInput(_) => StatusCode::BAD_REQUEST,
        FunctionError::InternalError(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

fn json_response<T: Serialize>(status: StatusCode, value: T) -> Response<Body> {
    match serde_json::to_vec(&value) {
        Ok(bytes) => Response::builder()
            .status(status)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(bytes))
            .unwrap(),
        Err(err) => {
            error!("failed to encode json response: {err}");
            error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to encode response",
            )
        }
    }
}

fn error_response(status: StatusCode, message: impl Into<String>) -> Response<Body> {
    let payload = json!({
        "success": false,
        "error": message.into(),
    });
    json_response(status, payload)
}
