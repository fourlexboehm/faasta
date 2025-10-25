use bytes::Bytes;
use compio::BufResult;
use compio::io::{AsyncWrite, AsyncWriteExt};
use compio::net::TcpListener;
use compio::runtime::spawn;
use compio::time::{timeout, sleep};
use compio::tls::TlsAcceptor;
use compio_dispatcher::Dispatcher;
use cyper_core::{CompioExecutor, CompioTimer, HyperStream};
use http::Response;
use http_body_util::{BodyExt, Full};
use hyper::Request;
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::server::conn::auto::Builder;
use std::convert::Infallible;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use tracing::{error, info, warn};

use crate::wasi_server::SERVER;
use crate::wasi_server::text_response;

/// Minimal HTTP redirect server that upgrades all traffic to HTTPS.
pub async fn run_http_server(http_listener: TcpListener, dispatcher: Arc<Dispatcher>) {
    if let Ok(addr) = http_listener.local_addr() {
        info!("HTTP redirect server listening on http://{}", addr);
    } else {
        info!("HTTP redirect server listening");
    }

    loop {
        let (mut stream, peer_addr) = match http_listener.accept().await {
            Ok(conn) => conn,
            Err(err) => {
                error!("Failed to accept redirect connection: {err}");
                continue;
            }
        };

        let dispatcher = dispatcher.clone();
        match dispatcher.dispatch(move || async move {
            let target = SERVER
                .get()
                .map(|server| format!("https://{}", server.base_domain))
                .unwrap_or_else(|| "https://faasta.xyz".to_string());

            let body = format!("Redirecting to {}\n", target);
            let response = format!(
                "HTTP/1.1 301 Moved Permanently\r\nLocation: {}\r\nContent-Length: {}\r\nContent-Type: text/plain; charset=utf-8\r\nConnection: close\r\n\r\n{}",
                target,
                body.len(),
                body
            );

            let response_bytes = response.into_bytes();

            let BufResult(write_res, _) = stream.write_all(response_bytes).await;
            if let Err(err) = write_res {
                error!("Failed to write redirect response to {}: {err}", peer_addr);
                return;
            }

            if let Err(err) = stream.shutdown().await {
                error!("Failed to shutdown redirect connection {}: {err}", peer_addr);
            }
        }) {
            Ok(handle) => {
                spawn(async move {
                    if let Err(err) = handle.await {
                        error!("Redirect task ended unexpectedly: {err}");
                    }
                })
                .detach();
            }
            Err(err) => {
                error!(?err, "Failed to dispatch redirect task");
            }
        }
    }
}

/// Runs the HTTPS server with improved resource management
pub async fn run_https_server(
    listener: TcpListener,
    tls_acceptor: TlsAcceptor,
    dispatcher: Arc<Dispatcher>,
) {
    info!("HTTPS server listening for connections");
    
    // Track consecutive errors for backoff
    let consecutive_errors = Arc::new(AtomicU64::new(0));
    let max_consecutive_errors = 10;
    
    // Track active connections to prevent resource exhaustion
    let active_connections = Arc::new(AtomicU64::new(0));
    let max_connections = 10000;

    loop {
        // Check if we're at connection limit
        let current_connections = active_connections.load(Ordering::Relaxed);
        if current_connections >= max_connections {
            warn!("At maximum connection limit ({}), applying backoff", max_connections);
            sleep(Duration::from_millis(100)).await;
            continue;
        }

        // Accept incoming connection
        let (stream, peer_addr) = match listener.accept().await {
            Ok(conn) => {
                // Reset error counter on successful accept
                consecutive_errors.store(0, Ordering::Relaxed);
                conn
            }
            Err(e) => {
                error!("Failed to accept connection: {}", e);
                
                // Increment error counter and apply backoff
                let error_count = consecutive_errors.fetch_add(1, Ordering::Relaxed) + 1;
                if error_count > max_consecutive_errors {
                    error!("Too many consecutive accept errors ({}), applying backoff", error_count);
                    sleep(Duration::from_millis(100 * error_count.min(50))).await;
                }
                
                // Brief sleep to prevent tight loop on persistent errors
                sleep(Duration::from_millis(10)).await;
                continue;
            }
        };
        
        // Increment active connections
        active_connections.fetch_add(1, Ordering::Relaxed);
        info!("Accepted connection from {} (active: {})", peer_addr, current_connections + 1);

        let tls_acceptor = tls_acceptor.clone();
        let dispatcher = dispatcher.clone();
        let consecutive_errors_clone = consecutive_errors.clone();
        let active_connections_clone = active_connections.clone();
        
        match dispatcher.dispatch(move || async move {
            // Ensure we decrement active connections on exit
            let _guard = ConnectionGuard { active_connections: active_connections_clone.clone() };
            
            // Perform TLS handshake with timeout
            match timeout(Duration::from_secs(10), tls_acceptor.accept(stream)).await {
                Ok(Ok(tls_stream)) => {
                    info!("TLS handshake successful with {}", peer_addr);
                    
                    // Reset error counter on successful handshake
                    consecutive_errors_clone.store(0, Ordering::Relaxed);

                    // Create a service function for handling HTTP requests
                    let header_timeout = Duration::from_secs(5);
                    let service = service_fn(move |req: Request<Incoming>| {
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
                                            let error_text = "Internal Server Error".to_string();
                                            let body = Full::new(Bytes::from(error_text))
                                                .map_err(|never: Infallible| match never {})
                                                .map_err(|_| wasmtime_wasi_http::bindings::http::types::ErrorCode::InternalError(None))
                                                .boxed();
                                            Ok(Response::builder()
                                                .status(500)
                                                .header("Content-Type", "text/plain")
                                                .body(body)
                                                .unwrap())
                                        }
                                    }
                                }
                            }
                        }
                    });

                    // Serve the HTTP connection directly with hyper using compio executor
                    let hyper_stream = HyperStream::new(tls_stream);
                    let mut builder = Builder::new(CompioExecutor);
                    builder.http1().timer(CompioTimer::default()).header_read_timeout(header_timeout);
                    builder.http2().timer(CompioTimer::default());
                    if let Err(err) = builder.serve_connection(hyper_stream, service).await {
                        error!("Error serving connection from {}: {}", peer_addr, err);
                    }
                }
                Ok(Err(e)) => {
                    error!("TLS handshake failed with {}: {}", peer_addr, e);
                    // Connection will be dropped here, closing the socket
                }
                Err(_) => {
                    error!("TLS handshake timed out with {}", peer_addr);
                    // Connection will be dropped here, closing the socket
                }
            }
        }) {
            Ok(handle) => {
                spawn(async move {
                    if let Err(err) = handle.await {
                        error!("HTTPS task ended unexpectedly: {err}");
                    }
                })
                .detach();
            }
            Err(err) => {
                error!(?err, "Failed to dispatch HTTPS task");
                // Decrement connection count since dispatch failed
                let active = active_connections.clone();
                active.fetch_sub(1, Ordering::Relaxed);
            }
        }
    }
}

// Helper struct to ensure connection count is decremented
struct ConnectionGuard {
    active_connections: Arc<AtomicU64>,
}

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        self.active_connections.fetch_sub(1, Ordering::Relaxed);
    }
}
