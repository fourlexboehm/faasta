use anyhow::{Result, anyhow, bail};
use bytes::Bytes;
use compio::runtime::spawn;
use compio::time::sleep;
use futures_util::future::Either;
use futures_util::{future::select, pin_mut};
use http_body_util::{BodyExt, Full};
use hyper::{Method, Request, Response, header::HOST};
use moka::future::Cache;
use once_cell::sync::OnceCell;
use std::sync::Arc;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use tokio::sync::oneshot;
use tracing::{debug, error, info};
use wasmtime::{
    Engine, Store,
    component::{Component, Linker, ResourceTable},
};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};
use wasmtime_wasi_http::bindings::ProxyPre;
use wasmtime_wasi_http::bindings::http::types::{ErrorCode, Scheme};
use wasmtime_wasi_http::body::HyperOutgoingBody;
use wasmtime_wasi_http::{WasiHttpCtx, WasiHttpView};

use crate::github_auth::GitHubAuth;
use crate::metrics::Timer;
use crate::rpc_service;

// Global server reference for cache management
pub static SERVER: OnceCell<FaastaServer> = OnceCell::new();

// Define the client state that holds ResourceTable, WasiCtx, and WasiHttpCtx
pub struct FaastaClientState {
    pub table: ResourceTable,
    pub wasi: WasiCtx,
    pub http: WasiHttpCtx,
}

pub static SHARED_LINKER: OnceCell<Linker<FaastaClientState>> = OnceCell::new();
pub static STORE_TEMPLATE_CTX: OnceCell<Box<dyn Fn() -> FaastaClientState + Send + Sync>> =
    OnceCell::new();

// Helper function to create text responses
pub fn text_response(status_code: u16, message: &str) -> Result<Response<HyperOutgoingBody>> {
    let body = Full::new(Bytes::from(message.to_string()))
        .map_err(|_| ErrorCode::InternalError(None))
        .boxed();

    Ok(Response::builder()
        .status(status_code)
        .header("Content-Type", "text/plain")
        .body(HyperOutgoingBody::new(body))?)
}

// Helper function to redirect to the main website
pub fn redirect_to_website() -> Result<Response<HyperOutgoingBody>> {
    let body = Full::new(Bytes::from("Redirecting to website..."))
        .map_err(|_| ErrorCode::InternalError(None))
        .boxed();

    Ok(Response::builder()
        .status(302)
        .header("Location", "https://website.faasta.xyz")
        .body(HyperOutgoingBody::new(body))?)
}

impl WasiView for FaastaClientState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

impl WasiHttpView for FaastaClientState {
    fn ctx(&mut self) -> &mut WasiHttpCtx {
        &mut self.http
    }

    fn table(&mut self) -> &mut ResourceTable {
        &mut self.table
    }
}

// Server state
pub struct FaastaServer {
    pub engine: Engine,
    pub metadata_db: sled::Db,
    pre_cache: Cache<String, Arc<ProxyPre<FaastaClientState>>>,
    pub base_domain: String,
    pub functions_dir: PathBuf,
    pub github_auth: GitHubAuth,
}

const BODY_READ_TIMEOUT: Duration = Duration::from_secs(10);

impl FaastaServer {
    pub async fn new(
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
            pre_cache: Cache::new(1000), // Limit to 1000 entries
            base_domain,
            functions_dir,
            github_auth,
        })
    }

    /// Remove a function from the pre_cache
    pub async fn remove_from_cache(&self, function_name: &str) {
        self.pre_cache.invalidate(function_name).await;
        debug!("Removed function '{}' from component cache", function_name);
    }

    pub async fn handle_request(
        &self,
        req: Request<hyper::body::Incoming>,
    ) -> Result<Response<HyperOutgoingBody>> {
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

            // Check for /v1/ prefix and handle API endpoints
            if path_parts.len() >= 2 && path_parts[1] == "v1" {
                // This is a v1 API request

                // Handle valid /v1/publish/{function_name} endpoint
                if path_parts.len() >= 4
                    && path_parts[2] == "publish"
                    && req.method() == Method::POST
                {
                    debug!("Processing v1 publish request");

                    // Extract function name from path
                    let function_name = path_parts[3].to_string();

                    // Get GitHub auth token from Authorization header
                    let github_auth_token = match req.headers().get("Authorization") {
                        Some(value) => {
                            match value.to_str() {
                                Ok(token) => {
                                    // Remove "Bearer " prefix if present
                                    let token = token.trim_start_matches("Bearer ").to_string();
                                    if token.is_empty() {
                                        return text_response(401, "Empty Authorization token");
                                    }
                                    token
                                }
                                Err(_) => {
                                    return text_response(
                                        401,
                                        "Invalid Authorization header format",
                                    );
                                }
                            }
                        }
                        None => return text_response(401, "Missing Authorization header"),
                    };

                    // Create service implementation
                    let service_impl = match rpc_service::create_service() {
                        Ok(service) => service,
                        Err(e) => {
                            error!("Failed to create function service: {}", e);
                            return text_response(500, "Internal server error");
                        }
                    };

                    // Read the body as WASM bytes
                    let body_future = http_body_util::BodyExt::collect(req.into_body());
                    let (timeout_tx, timeout_rx) = oneshot::channel();
                    spawn(async move {
                        sleep(BODY_READ_TIMEOUT).await;
                        let _ = timeout_tx.send(());
                    })
                    .detach();

                    pin_mut!(body_future);
                    let timeout_future = async {
                        let _ = timeout_rx.await;
                    };
                    pin_mut!(timeout_future);

                    let wasm_bytes = match select(body_future, timeout_future).await {
                        Either::Left((Ok(collected), _)) => collected.to_bytes().to_vec(),
                        Either::Left((Err(e), _)) => {
                            error!("Failed to read request body: {}", e);
                            return text_response(400, "Failed to read request body");
                        }
                        Either::Right(_) => {
                            error!("Timed out reading publish request body");
                            return text_response(408, "Request body timed out");
                        }
                    };

                    // Validate WASM bytes aren't empty
                    if wasm_bytes.is_empty() {
                        return text_response(400, "Empty WASM file");
                    }

                    // Call the service to publish the function
                    match service_impl
                        .publish_impl(wasm_bytes, function_name, github_auth_token)
                        .await
                    {
                        Ok(message) => {
                            let json = serde_json::json!({
                                "success": true,
                                "message": message
                            });

                            let body = Full::new(Bytes::from(json.to_string()))
                                .map_err(|_| ErrorCode::InternalError(None))
                                .boxed();

                            return Ok(Response::builder()
                                .status(200)
                                .header("Content-Type", "application/json")
                                .body(HyperOutgoingBody::new(body))
                                .unwrap());
                        }
                        Err(err) => {
                            let status_code = match &err {
                                faasta_interface::FunctionError::AuthError(_) => 401,
                                faasta_interface::FunctionError::NotFound(_) => 404,
                                faasta_interface::FunctionError::PermissionDenied(_) => 403,
                                faasta_interface::FunctionError::InvalidInput(_) => 400,
                                faasta_interface::FunctionError::InternalError(_) => 500,
                            };

                            error!("Publish operation failed: {err}");

                            let json = serde_json::json!({
                                "success": false,
                                "error": err.to_string()
                            });

                            let body = Full::new(Bytes::from(json.to_string()))
                                .map_err(|_| ErrorCode::InternalError(None))
                                .boxed();

                            return Ok(Response::builder()
                                .status(status_code)
                                .header("Content-Type", "application/json")
                                .body(HyperOutgoingBody::new(body))
                                .unwrap());
                        }
                    }
                } else {
                    // Invalid v1 path
                    return text_response(403, "Forbidden: Invalid API endpoint");
                }
            } else if path_parts.len() >= 2 && !path_parts[1].is_empty() {
                let function_name = path_parts[1].to_string();
                debug!(
                    "Processing path-based request for function: {}",
                    function_name
                );

                // Use direct function name approach
                let wasm_filename = format!("{function_name}.cwasm");
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
                    return text_response(404, &format!("Function '{function_name}' not found"));
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
            let wasm_filename = format!("{subdomain}.cwasm");
            debug!("Looking for WASM file: {}", wasm_filename);

            // Create a timer for this function call - will be moved to execute_function
            let function_path = self.functions_dir.join(&wasm_filename);
            if !function_path.exists() {
                debug!("Function not found at path: {:?}", function_path);
                return text_response(404, &format!("Function '{subdomain}' not found"));
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
        req: Request<hyper::body::Incoming>,
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
                wasi: WasiCtxBuilder::new().build(), // No stdio inheritance
                http: WasiHttpCtx::new(),
            })
        });

        // Use the template to create a store with similar configuration
        let mut client_state = store_template();

        // Update environment for this specific function
        client_state.wasi = WasiCtxBuilder::new()
            // Explicitly don't inherit stdio for security
            .env("FUNCTION_NAME", function_name)
            .build();

        // Get or load the ProxyPre
        let pre = self
            .get_or_load_proxy_pre(function_name, function_path)
            .await?;

        // Create store with client state
        let mut store = Store::new(pre.engine(), client_state);

        // Setup the response channel
        let (sender, receiver) = oneshot::channel();

        // Create the WASI HTTP request
        let wasi_req = store.data_mut().new_incoming_request(Scheme::Http, req)?;
        let wasi_resp_out = store.data_mut().new_response_outparam(sender)?;

        let proxy = pre.instantiate_async(&mut store).await?;

        // Spawn a task to handle the function execution
        let task = spawn(async move {
            proxy
                .wasi_http_incoming_handler()
                .call_handle(store, wasi_req, wasi_resp_out)
                .await?;
            Ok::<_, anyhow::Error>(())
        });

        // Wait for response with a 10-minute timeout
        match receiver.await {
            Ok(Ok(resp)) => Ok(resp),
            Ok(Err(err_code)) => {
                error!("Function returned error: {:?}", err_code);
                Err(anyhow!("Function error: {:?}", err_code))
            }
            Err(_) => match task.await {
                Ok(Ok(())) => bail!("Function did not set response"),
                Ok(Err(e)) => Err(e),
                Err(panic) => {
                    let message =
                        if let Some(msg) = panic.downcast_ref::<String>().map(|s| s.clone()) {
                            msg
                        } else if let Some(msg) = panic.downcast_ref::<&'static str>() {
                            (*msg).to_string()
                        } else {
                            "function task panicked".to_string()
                        };
                    Err(anyhow!(message))
                }
            },
        }
    }

    async fn get_or_load_proxy_pre(
        &self,
        function_name: &str,
        function_path: &PathBuf,
    ) -> Result<ProxyPre<FaastaClientState>> {
        let start_time = Instant::now();
        debug!(
            "get_or_load_proxy_pre called for function: {}",
            function_name
        );

        // First check if we have this pre-cached
        if let Some(cached) = self.pre_cache.get(function_name).await {
            let elapsed = start_time.elapsed();
            info!(
                "Proxy pre-cache hit for '{}', retrieved in {:?}",
                function_name, elapsed
            );
            return Ok((*cached).clone());
        }

        info!(
            "Proxy pre-cache miss for '{}', loading from file",
            function_name
        );
        let component_load_start = Instant::now();

        let component = unsafe { Component::deserialize_file(&self.engine, function_path) }?;
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
            // wasmtime_wasi::p3::add_to_linker_async(&mut linker).expect("Failed to add WASI to linker");
            wasmtime_wasi::p2::add_to_linker_async(&mut linker)
                .expect("Failed to add WASI to linker");
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
        info!("ProxyPre created for '{}' in {:?}", function_name, pre_time);

        // Cache it for future use
        self.pre_cache
            .insert(function_name.to_owned(), Arc::new(pre.clone()))
            .await;

        let total_elapsed = start_time.elapsed();
        info!(
            "get_or_load_proxy_pre complete for '{}' in {:?} (cache miss)",
            function_name, total_elapsed
        );

        Ok(pre)
    }
}
