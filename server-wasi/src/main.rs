#![warn(unused_extern_crates)]

use anyhow::{anyhow, bail, Context, Result};
use bytes::Bytes;
use clap::Parser;
use faasta_interface::FunctionService;
use http_body_util::{BodyExt, Full};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{body::Incoming, header::HOST, Request, Response};
mod cert_manager;
mod github_auth;
mod rpc_service;
use github_auth::GitHubAuth;
mod metrics;
use cert_manager::CertManager;
use metrics::Timer;
use tarpc::serde_transport as transport;
use tarpc::server::{BaseChannel, Channel};
use tarpc::tokio_serde::formats::Bincode;
use tarpc::tokio_util::codec::LengthDelimitedCodec;
// Removed unused imports from lers
use dashmap::DashMap;
use futures::prelude::*;
use once_cell::sync::OnceCell;
use rustls_pemfile::{certs, private_key};
use std::fs::File;
use std::io::BufReader;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio_rustls::rustls::ServerConfig;
use tokio_rustls::TlsAcceptor;
use tracing::{debug, error, info, Level};
use wasmtime::component::{Component, Linker, ResourceTable};
use wasmtime::{Config, Engine, Store};
use wasmtime_wasi::bindings::LinkOptions;
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiView};
use wasmtime_wasi_http::bindings::http::types::{ErrorCode, Scheme};
use wasmtime_wasi_http::bindings::ProxyPre;
use wasmtime_wasi_http::body::HyperOutgoingBody;
use wasmtime_wasi_http::io::TokioIo;
use wasmtime_wasi_http::{WasiHttpCtx, WasiHttpView};

// Global server reference for cache management
pub static SERVER: OnceCell<Arc<FaastaServer>> = OnceCell::new();

// Create a basic response with string content
fn text_response(status: u16, text: &str) -> Result<Response<HyperOutgoingBody>> {
    // Create a simple body with the provided text
    // Clone the text to ensure it's owned data that will live beyond this function
    let text_owned = text.to_string();
    let body = Full::new(Bytes::from(text_owned))
        .map_err(|_| ErrorCode::InternalError(None))
        .boxed();

    // Build and return the response
    Ok(Response::builder()
        .status(status)
        .header("Content-Type", "text/plain")
        .body(HyperOutgoingBody::new(body))?)
}

// Create a redirect response to website.faasta.xyz
fn redirect_to_website() -> Result<Response<HyperOutgoingBody>> {
    // Create a simple body with a redirect message
    let text_owned = "Redirecting to website.faasta.xyz...".to_string();
    let body = Full::new(Bytes::from(text_owned))
        .map_err(|_| ErrorCode::InternalError(None))
        .boxed();

    // Build and return the redirect response
    Ok(Response::builder()
        .status(302)
        .header("Location", "https://website.faasta.xyz")
        .header("Content-Type", "text/plain")
        .body(HyperOutgoingBody::new(body))?)
}

// Define the client state that holds ResourceTable, WasiCtx, and WasiHttpCtx
struct FaastaClientState {
    table: ResourceTable,
    wasi: WasiCtx,
    http: WasiHttpCtx,
    #[allow(dead_code)] // Kept for future telemetry/debugging
    function_name: String,
}

impl wasmtime_wasi::IoView for FaastaClientState {
    fn table(&mut self) -> &mut ResourceTable {
        &mut self.table
    }
}

impl WasiView for FaastaClientState {
    fn ctx(&mut self) -> &mut WasiCtx {
        &mut self.wasi
    }
}

impl WasiHttpView for FaastaClientState {
    fn ctx(&mut self) -> &mut WasiHttpCtx {
        &mut self.http
    }
}

// Server state
pub struct FaastaServer {
    engine: Engine,
    metadata_db: sled::Db,
    pre_cache: DashMap<String, ProxyPre<FaastaClientState>>,
    base_domain: String,
    functions_dir: PathBuf,
    github_auth: Arc<GitHubAuth>,
}

impl FaastaServer {
    async fn new(
        engine: Engine,
        metadata_db: sled::Db,
        base_domain: String,
        functions_dir: PathBuf,
    ) -> Result<Self> {
        // Initialize GitHub auth
        let github_auth = Arc::new(GitHubAuth::new(metadata_db.clone()).await?);

        Ok(Self {
            engine,
            metadata_db,
            pre_cache: DashMap::new(),
            base_domain,
            functions_dir,
            github_auth,
        })
    }

    /// Remove a function from the pre_cache
    pub fn remove_from_cache(&self, function_name: &str) {
        if self.pre_cache.contains_key(function_name) {
            self.pre_cache.remove(function_name);
            debug!("Removed function '{}' from component cache", function_name);
        }
    }

    async fn handle_request(&self, req: Request<Incoming>) -> Result<Response<HyperOutgoingBody>> {
        // Extract function name from subdomain
        let host_header = req.headers().get(HOST).and_then(|h| h.to_str().ok());

        // Check if it's the root domain
        if host_header.map(|h| h == self.base_domain).unwrap_or(false) {
            return redirect_to_website();
        }

        if let Some(host) = host_header {
            let expected_suffix = format!(".{}", self.base_domain);

            if !host.ends_with(&expected_suffix) {
                return redirect_to_website();
            }

            let subdomain = host.trim_end_matches(&expected_suffix);
            if subdomain.is_empty() || subdomain == host {
                return redirect_to_website();
            }

            debug!("Processing request for function: {}", subdomain);

            // Use direct function name approach
            let wasm_filename = format!("{}.wasm", subdomain);
            debug!("Looking for WASM file: {}", wasm_filename);

            // Create a timer for this function call
            let _timer = Timer::new(subdomain.to_string());
            let function_path = self.functions_dir.join(wasm_filename);
            if !function_path.exists() {
                return redirect_to_website();
            }

            // Get or load the ProxyPre
            let pre = self
                .get_or_load_proxy_pre(subdomain, &function_path)
                .await?;

            // Create store with client state
            let mut store = Store::new(
                pre.engine(),
                FaastaClientState {
                    table: ResourceTable::new(),
                    wasi: WasiCtxBuilder::new()
                        .inherit_stdio()
                        .env("FUNCTION_NAME", subdomain)
                        .build(),
                    http: WasiHttpCtx::new(),
                    function_name: subdomain.to_string(),
                },
            );

            // Setup the response channel
            let (sender, receiver) = tokio::sync::oneshot::channel();

            // Create the WASI HTTP request
            let wasi_req = store.data_mut().new_incoming_request(Scheme::Http, req)?;
            let wasi_resp_out = store.data_mut().new_response_outparam(sender)?;

            // Clone the pre so we can move it into the task
            let pre_clone = pre.clone();

            // Spawn task to execute the function
            let task = tokio::task::spawn(async move {
                let proxy = pre_clone.instantiate_async(&mut store).await?;
                proxy
                    .wasi_http_incoming_handler()
                    .call_handle(store, wasi_req, wasi_resp_out)
                    .await?;
                Ok::<_, anyhow::Error>(())
            });

            // Wait for response
            match receiver.await {
                Ok(Ok(resp)) => Ok(resp),
                Ok(Err(err_code)) => {
                    error!("Function returned error: {:?}", err_code);
                    Err(anyhow!("Function error: {:?}", err_code))
                }
                Err(_) => match task.await {
                    Ok(Ok(())) => bail!("Function did not set response"),
                    Ok(Err(e)) => Err(e),
                    Err(e) => Err(e.into()),
                },
            }
        } else {
            // No host header, redirect to website
            redirect_to_website()
        }
    }

    async fn get_or_load_proxy_pre(
        &self,
        function_name: &str,
        function_path: &PathBuf,
    ) -> Result<ProxyPre<FaastaClientState>> {
        // Check if we have this pre-cached
        if let Some(cached) = self.pre_cache.get(function_name) {
            return Ok(cached.value().clone());
        }

        // Load the component and create a linker
        let component = Component::from_file(&self.engine, function_path)?;
        let mut linker = Linker::new(&self.engine);

        // wasmtime_wasi::add_to_linker_async(&mut linker)?;
        // let link_options = self.run.compute_wasi_features();
        let options = LinkOptions::default();
        wasmtime_wasi::add_to_linker_with_options_async(&mut linker, &options)?;
        wasmtime_wasi_http::add_only_http_to_linker_async(&mut linker)?;
        // wasmtime_wasi::add_to_linker_async(&mut linker)?;
        // wasmtime_wasi::bindings::cli::environment::add_to_linker_get_host(&mut linker, |_| {})?;

        // Create the ProxyPre
        let pre = ProxyPre::new(linker.instantiate_pre(&component)?)?;

        // Cache it for future use
        self.pre_cache
            .insert(function_name.to_string(), pre.clone());

        Ok(pre)
    }
}

// We no longer need the convert_wasi_response function as we're using the direct response from wasmtime

#[derive(Parser, Debug, Clone)]
#[command(name = "server-wasi")]
#[command(about = "WASI HTTP Function Server", long_about = None)]
struct Args {
    /// Address to listen on (e.g., 0.0.0.0:443)
    #[arg(short, long, env = "LISTEN_ADDR", default_value = "0.0.0.0:443")]
    listen_addr: SocketAddr,

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

    /// Email address for Let's Encrypt
    #[arg(long, env = "LETSENCRYPT_EMAIL", default_value = "admin@faasta.xyz")]
    letsencrypt_email: String,

    /// Use Let's Encrypt staging environment (for testing)
    #[arg(long, env = "LETSENCRYPT_STAGING", default_value = "false")]
    letsencrypt_staging: bool,

    /// Auto-generate TLS certificate using Let's Encrypt
    #[arg(long, env = "AUTO_CERT", default_value = "true")]
    auto_cert: bool,

    /// Path to the SledDB database directory
    #[arg(long, env = "DB_PATH", default_value = "./data/db")]
    db_path: PathBuf,

    /// Path to the functions directory
    #[arg(long, env = "FUNCTIONS_PATH", default_value = "./functions")]
    functions_path: PathBuf,
}

async fn load_tls_config(args: &Args) -> Result<Arc<ServerConfig>> {
    // Load TLS certificate
    let cert_file = File::open(&args.tls_cert_path)
        .with_context(|| format!("Failed to open TLS cert file: {:?}", args.tls_cert_path))?;
    let mut cert_reader = BufReader::new(cert_file);
    let certs = certs(&mut cert_reader)
        .collect::<Result<Vec<_>, _>>()
        .context("Failed to parse TLS certificates")?;

    // Load TLS private key
    let key_file = File::open(&args.tls_key_path)
        .with_context(|| format!("Failed to open TLS key file: {:?}", args.tls_key_path))?;
    let mut key_reader = BufReader::new(key_file);
    let key = private_key(&mut key_reader)?.context("No private key found in TLS key file")?;

    // Build TLS config
    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .context("Failed to build TLS server config")?;

    Ok(Arc::new(config))
}

// The obtain_certificate function was removed as it's now handled by the CertManager

// Function to handle connections for tarpc
async fn run_server(mut quic_server: s2n_quic::Server, server: Arc<FaastaServer>) {
    while let Some(mut connection) = quic_server.accept().await {
        let server_clone = server.clone();
        tokio::spawn(async move {
            debug!("Accepted new connection");

            while let Ok(Some(stream)) = connection.accept_bidirectional_stream().await {
                let server_clone = server_clone.clone();
                tokio::spawn(async move {
                    debug!("Accepted new stream");
                    let framed = LengthDelimitedCodec::builder().new_framed(stream);
                    let transport = transport::new(framed, Bincode::default());

                    // Create a cloned service for this connection
                    let github_auth_clone = server_clone.github_auth.clone();
                    let service_clone = rpc_service::create_service_with_github_auth(
                        server_clone.functions_dir.clone(),
                        github_auth_clone,
                        server_clone.metadata_db.clone(),
                    )
                    .expect("Failed to create function service");

                    // Process this connection
                    let server_channel = BaseChannel::with_defaults(transport);
                    server_channel
                        .execute(service_clone.serve())
                        .for_each(|fut| {
                            tokio::spawn(fut);
                            async {}
                        })
                        .await;
                });
            }
        });
    }
}

// Removed unused spawn helper function

#[tokio::main]
async fn main() -> Result<()> {
    // Install default crypto provider for rustls
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install crypto provider");

    // Load environment variables from .env file if present
    let _ = dotenvy::dotenv();

    // Initialize tracing
    tracing_subscriber::fmt().with_max_level(Level::INFO).init();

    // Parse command-line arguments
    let args = Args::parse();

    // Ensure required directories exist
    std::fs::create_dir_all(&args.db_path)?;
    std::fs::create_dir_all(&args.functions_path)?;
    std::fs::create_dir_all(&args.certs_dir)?;

    // Setup certificate management
    if args.auto_cert {
        // Create CertManager instance for Porkbun
        let cert_manager = CertManager::new(
            args.base_domain.clone(),
            args.certs_dir.clone(),
            args.tls_cert_path.clone(),
            args.tls_key_path.clone(),
        );

        // Obtain or renew certificate if needed
        cert_manager
            .obtain_or_renew_certificate()
            .await
            .context("Failed to obtain/renew TLS certificate")?;
    }

    // Create a clone for use in the QUIC server task
    let args_clone = args.clone();

    info!(
        "Starting server-wasi with base domain: {}",
        args.base_domain
    );

    // Ensure metrics database directory exists
    let metrics_db_path =
        std::env::var("METRICS_DB_PATH").unwrap_or_else(|_| "./data/metrics".to_string());
    std::fs::create_dir_all(
        std::path::Path::new(&metrics_db_path)
            .parent()
            .unwrap_or_else(|| std::path::Path::new(".")),
    )?;

    // Open/create component cache database
    let pre_cache = sled::open(&args.db_path)?;

    // Configure Wasmtime engine

    let mut config = Config::new();
    config.async_support(true);
    config.wasm_component_model(true);

    // Create the engine
    let engine = Engine::new(&config)?;

    // Create server
    let server_instance = Arc::new(
        FaastaServer::new(
            engine,
            pre_cache,
            args.base_domain.clone(),
            args.functions_path.clone(),
        )
        .await?,
    );

    // Store server in global OnceCell for cache management
    let _ = SERVER.set(server_instance.clone());

    // Spawn a background task to flush metrics to DB every hour
    metrics::spawn_periodic_flush(60 * 60);

    // Load TLS configuration
    let tls_config = load_tls_config(&args).await?;

    let tls_acceptor = TlsAcceptor::from(tls_config.clone());

    // Start listening for connections
    let listener = TcpListener::bind(&args.listen_addr)
        .await
        .with_context(|| format!("Failed to bind to {}", args.listen_addr))?;
    info!("Listening on https://{}", args.listen_addr);

    // Start tarpc service for function management
    let server_clone = server_instance.clone();
    tokio::spawn(async move {
        let addr = "0.0.0.0:4433".parse::<std::net::SocketAddr>().unwrap();

        // Configure server with the TLS certs
        let quic_server = s2n_quic::Server::builder()
            .with_tls((
                Path::new(&args_clone.tls_cert_path),
                Path::new(&args_clone.tls_key_path),
            ))
            .map_err(|e| anyhow::anyhow!("Failed to set up TLS: {:?}", e))
            .expect("Failed to set up TLS")
            .with_io(addr)
            .map_err(|e| anyhow::anyhow!("Failed to set up IO: {:?}", e))
            .expect("Failed to set up IO")
            .start()
            .map_err(|e| anyhow::anyhow!("Failed to start server: {:?}", e))
            .expect("Failed to start server");

        info!("RPC service listening on {}", addr);

        // Process connections
        run_server(quic_server, server_clone).await;
    });
    // Main server loop
    loop {
        // Accept incoming connection
        let (stream, peer_addr) = match listener.accept().await {
            Ok(conn) => conn,
            Err(e) => {
                error!("Failed to accept connection: {}", e);
                continue;
            }
        };
        info!("Accepted connection from {}", peer_addr);

        // Clone server and acceptor for this connection
        let server = server_instance.clone();
        let tls_acceptor = tls_acceptor.clone();

        // Handle connection in a new task
        tokio::spawn(async move {
            // Perform TLS handshake
            match tls_acceptor.accept(stream).await {
                Ok(tls_stream) => {
                    info!("TLS handshake successful with {}", peer_addr);

                    // Create a service function for handling HTTP requests
                    let service = service_fn(move |req: Request<Incoming>| {
                        let server = server.clone();
                        async move {
                            match server.handle_request(req).await {
                                Ok(response) => Ok::<_, anyhow::Error>(response),
                                Err(e) => {
                                    error!("Error handling request: {}", e);
                                    // Return a generic 500 error response
                                    match text_response(500, "Internal Server Error") {
                                        Ok(resp) => Ok(resp),
                                        Err(err) => {
                                            error!("Failed to create error response: {}", err);
                                            // Fall back to a minimal hard-coded response if everything else fails
                                            let error_text = "Internal Server Error".to_string();
                                            let body = Full::new(Bytes::from(error_text))
                                                .map_err(|_| ErrorCode::InternalError(None))
                                                .boxed();
                                            Ok(Response::builder()
                                                .status(500)
                                                .header("Content-Type", "text/plain")
                                                .body(HyperOutgoingBody::new(body))
                                                .unwrap())
                                        }
                                    }
                                }
                            }
                        }
                    });

                    // Serve the HTTP connection directly with hyper
                    if let Err(err) = http1::Builder::new()
                        .serve_connection(TokioIo::new(tls_stream), service)
                        .await
                    {
                        // Only log errors that aren't from client disconnects
                        if !err.is_closed() && !err.is_canceled() {
                            error!("Error serving connection from {}: {}", peer_addr, err);
                        }
                    }
                }
                Err(e) => {
                    error!("TLS handshake failed with {}: {}", peer_addr, e);
                }
            }
        });
    }
}
