#![warn(unused_extern_crates)]

use anyhow::{Context, Result};
use clap::Parser;
use std::fs;
use std::net::SocketAddr;
mod cert_manager;
mod github_auth;
mod http;
mod metrics;
mod quic;
mod rpc_service;
mod wasi_server;
use cert_manager::CertManager;
use wasi_server::SERVER;

// use once_cell::sync::OnceCell;

use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio_rustls::rustls::ServerConfig;
use tokio_rustls::TlsAcceptor;
use tracing::{error, info, Level};
use wasmtime::{Config, Engine, InstanceAllocationStrategy, OptLevel, PoolingAllocationConfig};

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

    // Parse the certificates
    let certs = rustls_pemfile::certs(&mut cert_reader).collect::<Result<Vec<_>, _>>()?;

    // Load TLS private key
    let key_file = File::open(&args.tls_key_path)
        .with_context(|| format!("Failed to open TLS key file: {:?}", args.tls_key_path))?;
    let mut key_reader = BufReader::new(key_file);

    // Parse the private key
    let key = rustls_pemfile::private_key(&mut key_reader)?
        .ok_or_else(|| anyhow::anyhow!("No private key found in TLS key file"))?;

    // Build TLS config
    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .context("Failed to build TLS server config")?;

    Ok(Arc::new(config))
}

// HTTP to HTTPS redirection using Axum framework
// Note: run_http_server function has been moved to the http module

#[tokio::main]
async fn main() -> anyhow::Result<()> {
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
        let cert_manager = Arc::new(CertManager::new(
            args.base_domain.clone(),
            args.certs_dir.clone(),
            args.tls_cert_path.clone(),
            args.tls_key_path.clone(),
        ));

        cert_manager
            .obtain_or_renew_certificate()
            .await
            .context("Failed to obtain/renew TLS certificate")?;
        
        // Spawn periodic certificate download (every 7 days)
        info!("Starting periodic certificate downloads every 7 days");
        cert_manager.spawn_periodic_renewal();
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
            let wasm = fs::read(&path).unwrap();
            let cwasm = engine.precompile_component(&wasm).unwrap();
            fs::write(path.with_extension("cwasm"), cwasm).unwrap();
        }

        info!("Precompilation complete");
        Ok(())
    }


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
    pool.max_memory_size(1 << 28); // 256 MiB
    pool.total_tables(100);
    pool.table_elements(5000);
    pool.total_core_instances(100);
    config.allocation_strategy(InstanceAllocationStrategy::Pooling(pool));

    // Enable module caching to speed up startup time
    config.cache_config_load_default()?;

    // Set compilation settings
    config.cranelift_opt_level(OptLevel::Speed);

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
    let server_instance = wasi_server::FaastaServer::new(
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

    // Start HTTP server as a separate tokio task
    tokio::spawn(http::run_http_server(http_listener));

    // Start listening for HTTPS connections
    let listener = TcpListener::bind(&args.listen_addr)
        .await
        .with_context(|| format!("Failed to bind to {}", args.listen_addr))?;
    info!("Listening on https://{}", args.listen_addr);

    // Start RPC service for function management on a dedicated compio runtime
    let rpc_address = String::from("0.0.0.0:4433");
    if let Err(e) = quic::spawn_rpc_server(
        args.tls_cert_path.clone(),
        args.tls_key_path.clone(),
        rpc_address,
    ) {
        error!("Failed to spawn RPC server: {e}");
    }

    // Run HTTPS server in the main thread
    http::run_https_server(listener, tls_acceptor).await;
    Ok(())
}
