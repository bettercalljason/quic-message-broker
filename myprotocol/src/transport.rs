use std::io;
use std::path::Path;
use std::{fs, sync::Arc};

use anyhow::bail;
use anyhow::Context;
use anyhow::Result;
use bytes::BytesMut;
use quinn::crypto::rustls::QuicServerConfig;
use quinn::{Connection, Endpoint, Incoming};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio::io::AsyncReadExt;
use tracing::info;

use crate::{error::ServerError, protocol::ProtocolHandler, state::ServerState, Opt};

const ALPN_QUIC_HTTP: &[&[u8]] = &[b"hq-29"];

pub async fn run_quic_listener(
    options: Opt,
    server_state: Arc<ServerState>,
    handler: Arc<dyn ProtocolHandler + Send + Sync>,
) -> Result<()> {
    let endpoint = setup_quic(options).await?;
    accept_incoming(&endpoint, server_state, handler).await?;
    endpoint.wait_idle().await;
    Ok(())
}

async fn setup_quic(options: Opt) -> Result<Endpoint> {
    let (certs, key) = if let (Some(key_path), Some(cert_path)) = (&options.key, &options.cert) {
        let key = fs::read(key_path).context("failed to read private key")?;
        let key = if key_path.extension().is_some_and(|x| x == "der") {
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key))
        } else {
            rustls_pemfile::private_key(&mut &*key)
                .context("malformed PKCS #1 private key")?
                .ok_or_else(|| anyhow::Error::msg("no private keys found"))?
        };
        let cert_chain = fs::read(cert_path).context("failed to read certificate chain")?;
        let cert_chain = if cert_path.extension().is_some_and(|x| x == "der") {
            vec![CertificateDer::from(cert_chain)]
        } else {
            rustls_pemfile::certs(&mut &*cert_chain)
                .collect::<Result<_, _>>()
                .context("invalid PEM-encoded certificate")?
        };

        (cert_chain, key)
    } else {
        let dirs = directories_next::ProjectDirs::from("org", "quinn", "quinn-examples").unwrap();
        let path = dirs.data_local_dir();
        let cert_path = path.join("cert.der");
        let key_path = path.join("key.der");
        let (cert, key) = match fs::read(&cert_path).and_then(|x| Ok((x, fs::read(&key_path)?))) {
            Ok((cert, key)) => (
                CertificateDer::from(cert),
                PrivateKeyDer::try_from(key).map_err(anyhow::Error::msg)?,
            ),
            Err(ref e) if e.kind() == io::ErrorKind::NotFound => {
                info!("generating self-signed certificate");
                let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
                let key = PrivatePkcs8KeyDer::from(cert.key_pair.serialize_der());
                let cert = cert.cert.into();
                fs::create_dir_all(path).context("failed to create certificate directory")?;
                fs::write(&cert_path, &cert).context("failed to write certificate")?;
                fs::write(&key_path, key.secret_pkcs8_der())
                    .context("failed to write private key")?;
                (cert, key.into())
            }
            Err(e) => {
                bail!("failed to read certificate: {}", e);
            }
        };

        (vec![cert], key)
    };

    let mut server_crypto = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)?;
    server_crypto.alpn_protocols = ALPN_QUIC_HTTP.iter().map(|&x| x.into()).collect();
    if options.keylog {
        server_crypto.key_log = Arc::new(rustls::KeyLogFile::new());
    }

    let mut server_config =
        quinn::ServerConfig::with_crypto(Arc::new(QuicServerConfig::try_from(server_crypto)?));
    let transport_config = Arc::get_mut(&mut server_config.transport).unwrap();
    transport_config.max_concurrent_uni_streams(0_u8.into());

    let endpoint = quinn::Endpoint::server(server_config, options.listen)?;
    eprintln!("listening on {}", endpoint.local_addr()?);

    Ok(endpoint)
}

async fn accept_incoming(
    endpoint: &Endpoint,
    server_state: Arc<ServerState>,
    handler: Arc<dyn ProtocolHandler + Send + Sync>,
) -> Result<(), ServerError> {
    while let Some(conn) = endpoint.accept().await {
        let new_conn = conn.await.map_err(|e| {
            todo!();
        })?;
        let server_state = server_state.clone();
        let handler = handler.clone();
        tokio::spawn(async move {
            handle_connection(new_conn, server_state, handler)
                .await
                .unwrap_or_else(|e| {
                    todo!();
                })
        });
    }
    Ok(())
}

async fn handle_connection(
    conn: Connection,
    server_state: Arc<ServerState>,
    handler: Arc<dyn ProtocolHandler + Send + Sync>,
) -> Result<(), ServerError> {
    while let Ok((mut send_stream, mut recv_stream)) = conn.accept_bi().await {
        let mut buf = BytesMut::new();

        // Read data from recv_stream
        let n = recv_stream
            .read_buf(&mut buf)
            .await
            .unwrap_or_else(|e| todo!());

        if n == 0 {
            // Stream closed (EOF)
            break;
        }

        handler
            .handle_bytes(&mut buf, &server_state, &mut send_stream)
            .await?;
    }
    Ok(())
}
