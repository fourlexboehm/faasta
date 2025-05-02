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
use wasmtime::{Config, Engine, InstanceAllocationStrategy, PoolingAllocationConfig, Store};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiView};
use wasmtime_wasi_http::bindings::http::types::{ErrorCode, Scheme};
use wasmtime_wasi_http::bindings::ProxyPre;
use wasmtime_wasi_http::body::HyperOutgoingBody;
use wasmtime_wasi_http::io::TokioIo;
use wasmtime_wasi_http::{WasiHttpCtx, WasiHttpView};
// Add Axum imports for HTTP redirection
use axum::BoxError;
use axum::{extract::Host, http::uri::Authority, response::Redirect, Router};
use hyper::Uri;

// Global server reference for cache management
pub static SERVER: OnceCell<FaastaServer> = OnceCell::new();

// Define the client state that holds ResourceTable, WasiCtx, and WasiHttpCtx
pub struct FaastaClientState {
    table: ResourceTable,
    wasi: WasiCtx,
    http: WasiHttpCtx,
}

pub static SHARED_LINKER: OnceCell<Linker<FaastaClientState>> = OnceCell::new();
pub static STORE_TEMPLATE_CTX: OnceCell<Box<dyn Fn() -> FaastaClientState + Send + Sync>> =
    OnceCell::new();

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
    github_auth: GitHubAuth,
}

impl FaastaServer {

    async fn new(
        engine: Engine,
        metadata_db: sled::Db,
        base_domain: String,
        functions_dir: PathBuf,
    ) -> Result<Self> {
        // Initialize GitHub auth
        let github_auth = GitHubAuth::new(metadata_db.clone()).await?;

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
        // Extract function name from subdomain or path
        let host_header = req
            .headers()
            .get(HOST)
            .and_then(|h| h.to_str().ok())
            .map(|s| s.to_string());
        let path = req.uri().path().to_string();

        debug!("Handling request with path: {}", path);

        // Check if it's the root domain or local development host
        if host_header
            .as_deref()
            .map(|h| {
                h == self.base_domain || h.starts_with("localhost") || h.starts_with("127.0.0.1")
            })
            .unwrap_or(false)
        {
            debug!("Processing request on root domain: {}", self.base_domain);
            // Root domain with no subdomain - try to route based on path
            let path_str = path.as_str();
            let path_parts: Vec<&str> = path_str.split('/').collect();

            if path_parts.len() >= 2 && !path_parts[1].is_empty() {
                let function_name = path_parts[1].to_string();
                debug!(
                    "Processing path-based request for function: {}",
                    function_name
                );

                // Use direct function name approach
                let wasm_filename = format!("{}.wasm", function_name);
                debug!("Looking for WASM file: {}", wasm_filename);

                // Create a timer for this function call - will be moved to execute_function
                let function_path = self.functions_dir.join(&wasm_filename);

                // Debug logging to track function path
                if function_path.exists() {
                    debug!("Found function at path: {:?}", function_path);
                    // Create a new path to remove the /{function_name} prefix
                    let new_path = if path_parts.len() > 2 {
                        // Keep the rest of the path
                        format!("/{}", path_parts[2..].join("/"))
                    } else {
                        // Just the root
                        "/".to_string()
                    };

                    debug!("Rewriting path to: {}", new_path);

                    // Build a new request with the modified path
                    let mut builder = Request::builder()
                        .method(req.method().clone())
                        .uri(new_path)
                        .version(req.version());

                    // Copy all headers
                    for (name, value) in req.headers() {
                        builder = builder.header(name, value);
                    }

                    let (_, body) = req.into_parts();
                    let new_req = builder.body(body)?;

                    return self
                        .execute_function(new_req, &function_name, &function_path)
                        .await;
                } else {
                    debug!("Function not found at path: {:?}", function_path);
                    // If we're looking for a specific function but it doesn't exist, return a 404
                    return text_response(404, &format!("Function '{}' not found", function_name));
                }
            }

            // No function found in path, redirect to website
            debug!("No function specified in path, redirecting to website");
            return redirect_to_website();
        }

        if let Some(host) = &host_header {
            let expected_suffix = format!(".{}", self.base_domain);
            debug!("Checking host: {} for subdomain routing", host);

            if !host
                .to_lowercase()
                .ends_with(&expected_suffix.to_lowercase())
                && !host.starts_with("localhost")
                && !host.starts_with("127.0.0.1")
            {
                debug!(
                    "Host doesn't end with expected suffix: {} and is not a local development host",
                    expected_suffix
                );
                return redirect_to_website();
            }

            // Use string view instead of cloning
            let subdomain = host.trim_end_matches(&expected_suffix);
            if subdomain.is_empty() || subdomain == *host {
                debug!("Empty subdomain or hostname equals subdomain, redirecting");
                return redirect_to_website();
            }

            debug!("Processing subdomain request for function: {}", subdomain);

            // Use direct function name approach - only format once
            let wasm_filename = format!("{}.wasm", subdomain);
            debug!("Looking for WASM file: {}", wasm_filename);

            // Create a timer for this function call - will be moved to execute_function
            let function_path = self.functions_dir.join(&wasm_filename);
            if !function_path.exists() {
                debug!("Function not found at path: {:?}", function_path);
                return text_response(404, &format!("Function '{}' not found", subdomain));
            }

            // Execute the function
            debug!("Executing function from subdomain route");
            return self.execute_function(req, subdomain, &function_path).await;
        } else {
            // No host header, redirect to website
            debug!("No host header found, redirecting to website");
            redirect_to_website()
        }
    }
    async fn execute_function(
        &self,
        req: Request<Incoming>,
        function_name: &str,
        function_path: &PathBuf,
    ) -> Result<Response<HyperOutgoingBody>> {
        let _timer = Timer::new(function_name.to_string());

        debug!(
            "Executing function: {} [path: {:?}]",
            function_name, function_path
        );

        // Initialize a store template function if not already done
        let store_template = STORE_TEMPLATE_CTX.get_or_init(|| {
            // This template function will be used to create a similarly configured store each time
            Box::new(move || FaastaClientState {
                table: ResourceTable::new(),
                wasi: WasiCtxBuilder::new().inherit_stdio().build(),
                http: WasiHttpCtx::new(),
            })
        });

        // Use the template to create a store with similar configuration
        let mut client_state = store_template();

        // Update environment for this specific function
        client_state.wasi = WasiCtxBuilder::new()
            // .inherit_stdio()
            .env("FUNCTION_NAME", function_name)
            .build();

        // Get or load the ProxyPre
        let pre = self
            .get_or_load_proxy_pre(function_name, function_path)
            .await?;

        // Create store with client state
        let mut store = Store::new(pre.engine(), client_state);

        // Setup the response channel
        let (sender, receiver) = tokio::sync::oneshot::channel();

        // Create the WASI HTTP request
        let wasi_req = store.data_mut().new_incoming_request(Scheme::Http, req)?;
        let wasi_resp_out = store.data_mut().new_response_outparam(sender)?;

        let proxy = pre.instantiate_async(&mut store).await?;

        // Spawn a task to handle the function execution
        let task = tokio::task::spawn(async move {
            proxy
                .wasi_http_incoming_handler()
                .call_handle(store, wasi_req, wasi_resp_out)
                .await?;
            Ok::<_, anyhow::Error>(())
        });

        // Wait for response with a 10-minute timeout
        match tokio::time::timeout(std::time::Duration::from_secs(600), receiver).await {
            Ok(receiver_result) => match receiver_result {
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
            },
            Err(_) => {
                error!("Function execution timed out after 10 minutes");
                Err(anyhow!("Function execution timed out after 10 minutes"))
            }
        }
    }

    async fn get_or_load_proxy_pre(
        &self,
        function_name: &str,
        function_path: &PathBuf,
    ) -> Result<ProxyPre<FaastaClientState>> {
        use tracing::{debug, info};
        use std::time::Instant;
        
        let start_time = Instant::now();
        debug!("get_or_load_proxy_pre called for function: {}", function_name);
        
        // First check if we have this pre-cached without cloning the key
        if let Some(cached) = self.pre_cache.get(function_name) {
            let elapsed = start_time.elapsed();
            info!(
                "Proxy pre-cache hit for '{}', retrieved in {:?}",
                function_name, elapsed
            );
            return Ok(cached.value().clone());
        }
        
        info!("Proxy pre-cache miss for '{}', loading from file", function_name);
        let component_load_start = Instant::now();
        
        // Load the component
        let component = Component::from_file(&self.engine, function_path)?;
        let component_load_time = component_load_start.elapsed();
        info!(
            "Component loaded for '{}' in {:?}",
            function_name, component_load_time
        );

        // Get the shared linker or create it once
        let linker_start = Instant::now();
        let linker = SHARED_LINKER.get_or_init(|| {
            info!("Initializing shared linker (first time)");
            let mut linker = Linker::new(&self.engine);

            // Set up WASI and WASI-HTTP definitions - only needs to be done once
            wasmtime_wasi::add_to_linker_async(&mut linker).expect("Failed to add WASI to linker");
            wasmtime_wasi_http::add_only_http_to_linker_async(&mut linker)
                .expect("Failed to add WASI-HTTP to linker");

            linker
        });
        let linker_time = linker_start.elapsed();
        if linker_time.as_millis() > 1 {
            info!("Linker initialization took {:?}", linker_time);
        }
        
        // Create the pre-instantiated component
        let pre_start = Instant::now();
        let pre = ProxyPre::new(linker.instantiate_pre(&component)?)?;
        let pre_time = pre_start.elapsed();
        info!(
            "ProxyPre created for '{}' in {:?}",
            function_name, pre_time
        );

        // Cache it for future use - only clone the string once when inserting
        self.pre_cache
            .insert(function_name.to_string(), pre.clone());
        
        let total_elapsed = start_time.elapsed();
        info!(
            "get_or_load_proxy_pre complete for '{}' in {:?} (cache miss)",
            function_name, total_elapsed
        );

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

// Function to handle connections for tarpc
async fn run_rpc_server(mut quic_server: s2n_quic::Server) {
    while let Some(mut connection) = quic_server.accept().await {
        // Clone the server Arc once for each connection

        tokio::spawn(async move {
            debug!("Accepted new connection");

            while let Ok(Some(stream)) = connection.accept_bidirectional_stream().await {

                tokio::spawn(async move {
                    debug!("Accepted new stream");
                    let framed = LengthDelimitedCodec::builder().new_framed(stream);
                    let transport = transport::new(framed, Bincode::default());

                    let service = rpc_service::create_service()
                    .expect("Failed to create function service");

                    // Process this connection
                    // Use default configuration but with a longer context deadline
                    let server_channel = BaseChannel::with_defaults(transport);

                    // Use a reference to the service to call serve()
                    server_channel
                        .execute(service.serve())
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

// HTTP to HTTPS redirection using Axum framework
async fn run_http_redirect_server(http_listener: TcpListener) {
    info!("HTTP redirect server listening for connections");

    // Create a function to convert HTTP URLs to HTTPS
    let make_https = |host: &str, uri: Uri, https_port: u16| -> Result<Uri, BoxError> {
        let mut parts = uri.into_parts();

        parts.scheme = Some(axum::http::uri::Scheme::HTTPS);

        if parts.path_and_query.is_none() {
            parts.path_and_query = Some("/".parse().unwrap());
        }

        let authority: Authority = host.parse()?;
        let bare_host = match authority.port() {
            Some(port_struct) => authority
                .as_str()
                .strip_suffix(port_struct.as_str())
                .unwrap()
                .strip_suffix(':')
                .unwrap(), // if authority.port() is Some(port) then we can be sure authority ends with :{port}
            None => authority.as_str(),
        };

        parts.authority = Some(format!("{bare_host}:{https_port}").parse()?);

        Ok(Uri::from_parts(parts)?)
    };

    // Get the local port this listener is bound to
    let listener_addr = http_listener.local_addr().unwrap();

    // Determine HTTPS port (default to 443)
    let https_port = 443;

    // Create the redirect handler
    let redirect = move |Host(host): Host, uri: Uri| async move {
        match make_https(&host, uri, https_port) {
            Ok(uri) => Ok(Redirect::permanent(&uri.to_string())),
            Err(error) => {
                tracing::warn!(%error, "failed to convert URI to HTTPS");
                Err(axum::http::StatusCode::BAD_REQUEST)
            }
        }
    };

    // Create Axum router with the redirect handler
    let app = Router::new().fallback(redirect);

    // Start the Axum HTTP server
    info!(
        "HTTP redirect service listening on http://{}",
        listener_addr
    );

    // Serve with the existing TcpListener
    axum::serve(http_listener, app).await.unwrap();
}

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

            cert_manager
                .obtain_or_renew_certificate()
                .await
                .context("Failed to obtain/renew TLS certificate")?;
    }

    // Pre-compile available functions to improve startup time
    async fn precompile_functions(engine: &Engine, functions_dir: &Path) -> Result<()> {
        info!("Pre-compiling functions...");

        let function_files = std::fs::read_dir(functions_dir)?
            .filter_map(|entry| {
                let entry = entry.ok()?;
                let path = entry.path();
                if path.extension().and_then(|ext| ext.to_str()) == Some("wasm") {
                    Some(path)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();

        // Log how many functions we're going to precompile
        info!("Found {} functions to precompile", function_files.len());

        // Precompile each function
        for path in function_files {
            let filename = path.file_name().unwrap().to_string_lossy();
            info!("Precompiling function: {}", filename);
            let _ = Component::from_file(engine, &path)?;
        }

        info!("Precompilation complete");
        Ok(())
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
    let metadata_db = sled::open(&args.db_path)?;
    let mut config = Config::default();
    config.async_support(true);
    config.wasm_component_model(true);
    config.memory_init_cow(true);
    let mut pool = PoolingAllocationConfig::new();
    pool.total_memories(100);
    pool.max_memory_size(1 << 31); // 2 GiB
    pool.total_tables(100);
    pool.table_elements(5000);
    pool.total_core_instances(100);
    config.allocation_strategy(InstanceAllocationStrategy::Pooling(pool));


    // Enable module caching to speed up startup time
    config.cache_config_load_default()?;
    // config.cac

    // Set compilation settings
    config.cranelift_opt_level(wasmtime::OptLevel::Speed);

    // Enable parallel compilation if available
    config.parallel_compilation(true);

    // Precompile modules ahead of time
    config.strategy(wasmtime::Strategy::Cranelift);

    // Create the engine
    let engine = Engine::new(&config)?;

    // // Precompile functions
    if let Err(e) = precompile_functions(&engine, &args.functions_path).await {
        error!("Error precompiling functions: {}", e);
    }

    // Create server
    let server_instance =
        FaastaServer::new(
            engine,
            metadata_db,
            args.base_domain.clone(),
            args.functions_path.clone(),
        )
        .await?;

    // Store server in global OnceCell for cache management
    let _ = SERVER.set(server_instance);

    // Spawn a background task to flush metrics to DB
    metrics::spawn_periodic_flush(60 * 30);

    // Load TLS configuration with timing
    info!("Loading TLS configuration...");
    let tls_timer = metrics::Timer::new("tls_config_loading".to_string());
    let tls_config = load_tls_config(&args).await?;
    drop(tls_timer); // Explicitly drop to record timing
    info!("TLS configuration loaded");

    let tls_acceptor = TlsAcceptor::from(tls_config.clone());

    // Start listening for HTTP connections (for redirects)
    let http_listener = TcpListener::bind(&args.http_listen_addr)
        .await
        .with_context(|| format!("Failed to bind to {}", args.http_listen_addr))?;
    info!(
        "HTTP redirect service listening on http://{}",
        args.http_listen_addr
    );

    // Start HTTP redirect server as a separate tokio task
    tokio::spawn(run_http_redirect_server(http_listener));

    // Start listening for HTTPS connections
    let listener = TcpListener::bind(&args.listen_addr)
        .await
        .with_context(|| format!("Failed to bind to {}", args.listen_addr))?;
    info!("Listening on https://{}", args.listen_addr);

    // Start tarpc service for function management
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
        run_rpc_server(quic_server).await;
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
        let tls_acceptor = tls_acceptor.clone();

        // Handle connection in a new task
        tokio::spawn(async move {
            // Perform TLS handshake
            match tls_acceptor.accept(stream).await {
                Ok(tls_stream) => {
                    info!("TLS handshake successful with {}", peer_addr);


                    // Create a service function for handling HTTP requests
                    // Clone the context for each request to avoid lifetime issues
                    let service = service_fn(move |req: Request<Incoming>| {
                        // Use the cloned context for each request

                        async move {
                            match SERVER.get().unwrap().handle_request(req).await {
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
