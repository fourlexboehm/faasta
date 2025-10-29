use anyhow::{Context, Result};
use bitrpc::ServerBuilder;
use compio::rustls::pki_types::{CertificateDer, PrivateKeyDer};
use compio_quic::ServerBuilder as QuicServerBuilder;
use faasta_interface::RpcRequestServiceWrapper;
use std::fs::File;
use std::io::BufReader;
use std::path::PathBuf;
use tracing::info;

use crate::rpc_service;

pub async fn run_rpc_server(
    tls_cert_path: PathBuf,
    tls_key_path: PathBuf,
    listen_addr: String,
) -> Result<()> {
    let cert_chain = load_certificates(&tls_cert_path)?;
    let private_key = load_private_key(&tls_key_path)?;

    let quic_config = QuicServerBuilder::new_with_single_cert(cert_chain, private_key)
        .context("failed to build QUIC config")?
        .with_alpn_protocols(&["h3"])
        .build();

    let service = rpc_service::create_service().context("failed to create RPC service")?;
    info!("RPC service listening on {listen_addr}");

    ServerBuilder::new(quic_config, listen_addr)
        .serve(RpcRequestServiceWrapper(service))
        .await
        .context("bitRPC server error")
}

fn load_certificates(path: &PathBuf) -> Result<Vec<CertificateDer<'static>>> {
    let file = File::open(path).with_context(|| format!("failed to open cert file: {path:?}"))?;
    let mut reader = BufReader::new(file);
    let certs = rustls_pemfile::certs(&mut reader)
        .collect::<std::result::Result<Vec<_>, _>>()
        .context("failed to read certificate chain")?;

    if certs.is_empty() {
        anyhow::bail!("no certificates found in {:?}", path);
    }

    Ok(certs)
}

fn load_private_key(path: &PathBuf) -> Result<PrivateKeyDer<'static>> {
    let file = File::open(path).with_context(|| format!("failed to open key file: {path:?}"))?;
    let mut reader = BufReader::new(file);
    rustls_pemfile::private_key(&mut reader)
        .context("failed to parse private key")?
        .ok_or_else(|| anyhow::anyhow!("no private key found in {:?}", path))
}
