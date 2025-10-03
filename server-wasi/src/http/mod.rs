use bytes::Bytes;
use compio::net::TcpListener;
use compio::runtime::spawn;
use compio::tls::TlsAcceptor;
use cyper_core::{CompioExecutor, HyperStream};
use http::Response;
use http_body_util::{BodyExt, Full};
use hyper::Request;
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::server::conn::auto::Builder;
use std::convert::Infallible;
use tracing::{error, info};

use crate::wasi_server::SERVER;
use crate::wasi_server::text_response;

/// Temporary stub while the redirect server is ported to compio.
#[allow(dead_code)]
pub async fn run_http_server(_http_listener: TcpListener) {
    info!("HTTP redirect server disabled during compio migration");
}

/// Runs the HTTPS server
pub async fn run_https_server(listener: TcpListener, tls_acceptor: TlsAcceptor) {
    info!("HTTPS server listening for connections");

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

        // Clone acceptor for this connection
        let tls_acceptor = tls_acceptor.clone();

        // Handle connection in a new task
        spawn(async move {
            // Perform TLS handshake
            match tls_acceptor.accept(stream).await {
                Ok(tls_stream) => {
                    info!("TLS handshake successful with {}", peer_addr);

                    // Create a service function for handling HTTP requests
                    let service = service_fn(move |req: Request<Incoming>| {
                        async move {
                            match SERVER.get().unwrap().handle_request(req).await {
                                Ok(response) => {
                                    // Just return the response directly - we'll handle any conversion issues
                                    // at a different layer if needed
                                    Ok::<_, anyhow::Error>(response)
                                }
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
                    if let Err(err) = Builder::new(CompioExecutor)
                        .serve_connection(hyper_stream, service)
                        .await
                    {
                        error!("Error serving connection from {}: {}", peer_addr, err);
                    }
                }
                Err(e) => {
                    error!("TLS handshake failed with {}: {}", peer_addr, e);
                }
            }
        })
        .detach();
    }
}
