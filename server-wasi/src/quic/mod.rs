use anyhow::{anyhow, Result};
use futures::StreamExt;
use std::path::PathBuf;
use tarpc::tokio_serde::formats::Bincode;
use tarpc::{
    serde_transport as transport,
    server::{BaseChannel, Channel},
};
use tokio_util::codec::LengthDelimitedCodec;
use tracing::{debug, info};

use crate::rpc_service;
use faasta_interface::FunctionService;

/// Configures and starts a QUIC server for RPC communication
pub async fn setup_quic_server(
    tls_cert_path: PathBuf,
    tls_key_path: PathBuf,
    rpc_address: &str,
) -> Result<()> {
    let addr = rpc_address
        .parse::<std::net::SocketAddr>()
        .map_err(|e| anyhow!("Invalid RPC address: {}", e))?;

    // Configure server with the TLS certs
    let quic_server = s2n_quic::Server::builder()
        .with_tls((tls_cert_path.as_path(), tls_key_path.as_path()))
        .map_err(|e| anyhow!("Failed to set up TLS: {:?}", e))?
        .with_io(addr)
        .map_err(|e| anyhow!("Failed to set up IO: {:?}", e))?
        .start()
        .map_err(|e| anyhow!("Failed to start server: {:?}", e))?;

    info!("RPC service listening on {}", addr);

    // Process connections
    run_rpc_server(quic_server).await;

    Ok(())
}

/// Runs the RPC server that handles QUIC connections
pub async fn run_rpc_server(mut quic_server: s2n_quic::Server) {
    while let Some(mut connection) = quic_server.accept().await {
        tokio::spawn(async move {
            debug!("Accepted new connection");

            while let Ok(Some(stream)) = connection.accept_bidirectional_stream().await {
                tokio::spawn(async move {
                    debug!("Accepted new stream");
                    let framed = LengthDelimitedCodec::builder().new_framed(stream);
                    let transport = transport::new(framed, Bincode::default());

                    let service =
                        rpc_service::create_service().expect("Failed to create function service");

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
