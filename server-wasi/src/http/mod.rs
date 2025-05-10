use anyhow::Result;
use axum::{
    extract::Host,
    http::uri::{Authority, Uri},
    response::Redirect,
    Router,
};
use bytes::Bytes;
use http::Response;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::Request;
use hyper_util::rt::TokioIo;
use std::convert::Infallible;
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tracing::{error, info};

use crate::wasi_server::text_response;
use crate::wasi_server::SERVER;

type BoxError = Box<dyn std::error::Error + Send + Sync>;

// Note: text_response and redirect_to_website functions have been moved to wasi_server module

/// Runs the HTTP server that redirects HTTP requests to HTTPS
pub async fn run_http_server(http_listener: TcpListener) {
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
                .unwrap(),
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
        tokio::spawn(async move {
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
