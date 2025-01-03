use anyhow::Result;
use clap::Parser;
use mqttbytes::QoS;
use myprotocol::{MqttProtocol, QuicTransport, ALPN_QUIC_HTTP};
use rustls::server::WebPkiClientVerifier;
use tokio::time::timeout;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;
use std::{fs, sync::Arc};
use tracing::{error, info};

use anyhow::Context;
use quinn::crypto::rustls::QuicServerConfig;
use quinn::{Connection, Endpoint, RecvStream, SendStream};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

use crate::handler::{BrokerConfig, PacketHandler};
use crate::state::ServerState;

#[derive(Parser, Debug)]
#[clap(name = "server-config")]
pub struct ServerConfig {
    /// file to log TLS keys to for debugging
    #[clap(long = "keylog")]
    pub keylog: bool,
    /// TLS private key in PEM format
    #[clap(
        short = 'k',
        long = "key",
        requires = "cert",
        default_value = "C:\\GitHub\\quic-message-broker\\tlsgen\\server\\key.der"
    )]
    pub key: PathBuf,
    /// TLS certificate in PEM format
    #[clap(
        short = 'c',
        long = "cert",
        requires = "key",
        default_value = "C:\\GitHub\\quic-message-broker\\tlsgen\\server\\cert.der"
    )]
    pub cert: PathBuf,
    /// TLS client certificate in PEM format to trust
    #[clap(
        long = "client_cert",
        default_value = "C:\\GitHub\\quic-message-broker\\tlsgen\\client\\cert.der"
    )]
    pub client_cert: PathBuf,
    /// Enable stateless retries
    #[clap(long = "stateless-retry")]
    pub stateless_retry: bool,
    /// Address to listen on
    #[clap(long = "listen", default_value = "[::1]:4433")]
    pub listen: SocketAddr,
    /// Client address to block
    #[clap(long = "block")]
    pub block: Option<SocketAddr>,
    /// Maximum number of concurrent connections to allow
    #[clap(long = "connection-limit")]
    pub connection_limit: Option<usize>,
}

pub async fn run_server(config: ServerConfig) -> Result<()> {
    let state = Arc::new(ServerState::new());

    let config2 = Arc::new(BrokerConfig {
        max_qos: QoS::AtMostOnce,
    });

    let endpoint = setup_quic(config).await?;
    accept_incoming(&endpoint, state, config2).await?;
    endpoint.wait_idle().await;

    Ok(())
}

async fn setup_quic(config: ServerConfig) -> Result<Endpoint> {
    let mut roots = rustls::RootCertStore::empty();
    roots.add(CertificateDer::from(fs::read(&config.client_cert)?))?;

    let key = fs::read(&config.key).context("failed to read private key")?;
    let key = if config.key.extension().is_some_and(|x| x == "der") {
        PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key))
    } else {
        rustls_pemfile::private_key(&mut &*key)
            .context("malformed PKCS #1 private key")?
            .ok_or_else(|| anyhow::Error::msg("no private keys found"))?
    };
    let certs = fs::read(&config.cert).context("failed to read certificate chain")?;
    let certs = if config.cert.extension().is_some_and(|x| x == "der") {
        vec![CertificateDer::from(certs)]
    } else {
        rustls_pemfile::certs(&mut &*certs)
            .collect::<Result<_, _>>()
            .context("invalid PEM-encoded certificate")?
    };

    let verifier = WebPkiClientVerifier::builder(roots.into()).build()?;

    let mut server_crypto = rustls::ServerConfig::builder()
        .with_client_cert_verifier(verifier)
        .with_single_cert(certs, key)?;
    server_crypto.alpn_protocols = ALPN_QUIC_HTTP.iter().map(|&x| x.into()).collect();
    if config.keylog {
        server_crypto.key_log = Arc::new(rustls::KeyLogFile::new());
    }

    let mut server_config =
        quinn::ServerConfig::with_crypto(Arc::new(QuicServerConfig::try_from(server_crypto)?));
    let transport_config = Arc::get_mut(&mut server_config.transport).unwrap();
    transport_config.max_concurrent_uni_streams(0_u8.into());

    let endpoint = quinn::Endpoint::server(server_config, config.listen)?;
    info!("Listening on {}", endpoint.local_addr()?);

    Ok(endpoint)
}

async fn accept_incoming(
    endpoint: &Endpoint,
    state: Arc<ServerState>,
    config: Arc<BrokerConfig>,
) -> Result<()> {
    while let Some(conn) = endpoint.accept().await {
        let new_conn = conn.await?;
        let state = state.clone();
        let config = config.clone();
        tokio::spawn(async move {
            if let Err(err) = handle_connection(new_conn, state, config).await {
                error!("Error handling connection: {:?}", err);
            }
        });
    }
    Ok(())
}

async fn handle_connection(
    conn: Connection,
    state: Arc<ServerState>,
    config: Arc<BrokerConfig>,
) -> Result<()> {
    while let Ok((send_stream, recv_stream)) = conn.accept_bi().await {
        let state = state.clone();
        let config = config.clone();

        tokio::spawn(async move {
            if let Err(e) = handle_stream(send_stream, recv_stream, state, config).await {
                error!("Error handling stream: {:?}", e);
            }
        });
    }
    Ok(())
}

async fn handle_stream(
    send: SendStream,
    recv: RecvStream,
    state: Arc<ServerState>,
    config: Arc<BrokerConfig>,
) -> Result<()> {
    let transport = QuicTransport::new(send, recv);
    let mut protocol = MqttProtocol::new(transport);

    let packet = protocol.recv_packet().await?;
    let client = PacketHandler::process_first_packet(packet, &config, &state).await?;

    if let Some((client_id, sender, mut receiver)) = client {
        loop {
            if let Ok(packet) = receiver.try_recv() {
                protocol.send_packet(packet).await?;
            }

            match timeout(Duration::from_millis(500), protocol.recv_packet()).await {
                Ok(recv) => {
                    match recv {
                        Ok(packet) => {
                            let should_close =
                                PacketHandler::process_packet(packet, &config, &state, &client_id, &sender).await?;
        
                            if should_close {
                                protocol.close_connection().await?;
                                break;
                            }
                        }
                        Err(e) => {
                            error!("Error handling packet: {:?}", e);
                            break;
                        }
                    }
                }
                Err(e) => continue
            }
        }
    }

    Ok(())
}
