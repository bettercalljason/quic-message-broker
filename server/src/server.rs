use anyhow::Result;
use clap::Parser;
use mqttbytes::QoS;
use quinn::crypto::rustls::QuicServerConfig;
use quinn::{Endpoint, RecvStream, SendStream};
use rustls::pki_types::PrivatePkcs8KeyDer;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use shared::{mqtt::MqttProtocol, transport::QuicTransport, transport::ALPN_QUIC_HTTP};
use std::fs;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;
use tracing::{error, info, info_span, Instrument};

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
        default_value = "C:\\GitHub\\quic-message-broker\\tlsgen\\key.der"
    )]
    pub key: PathBuf,
    /// TLS certificate in DER format
    #[clap(
        short = 'c',
        long = "cert",
        requires = "key",
        default_value = "C:\\GitHub\\quic-message-broker\\tlsgen\\cert.der"
    )]
    pub cert: PathBuf,
    /// Enable stateless retries
    #[clap(long = "stateless-retry")]
    pub stateless_retry: bool,
    /// Address to listen on
    #[clap(long = "listen", default_value = "[::1]:4433")]
    pub listen: SocketAddr,
    /// Maximum number of concurrent connections to allow
    #[clap(long = "connection-limit")]
    pub connection_limit: Option<usize>,
}

pub async fn run_server(config: ServerConfig) -> Result<()> {
    let state = Arc::new(ServerState::new());

    let endpoint = setup_quic(&config).await?;

    let config2 = Arc::new(BrokerConfig {
        max_qos: QoS::AtMostOnce,
        keep_alive: 10, // depends on QUIC TransportConfig keep_alive and idle_timeout configuration
    });

    while let Some(conn) = endpoint.accept().await {
        if config
            .connection_limit
            .is_some_and(|n| endpoint.open_connections() >= n)
        {
            info!("Refusing due to open connection limit");
            conn.refuse();
        } else {
            info!("Accepting connection");
            let fut = handle_connection(conn, state.clone(), config2.clone());
            tokio::spawn(async move {
                if let Err(e) = fut.await {
                    error!("Connection failed: {:?}", e);
                }
            });
        }
    }
    endpoint.wait_idle().await;

    Ok(())
}

async fn setup_quic(config: &ServerConfig) -> Result<Endpoint> {
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(fs::read(&config.key)?));
    let certs = vec![CertificateDer::from(fs::read(&config.cert)?)];

    let mut server_crypto = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)?;
    server_crypto.alpn_protocols = ALPN_QUIC_HTTP.iter().map(|&x| x.into()).collect();

    let mut server_config =
        quinn::ServerConfig::with_crypto(Arc::new(QuicServerConfig::try_from(server_crypto)?));
    let transport_config = Arc::get_mut(&mut server_config.transport).unwrap();
    transport_config.max_concurrent_uni_streams(0_u8.into());
    transport_config.max_concurrent_bidi_streams(1_u8.into());

    let endpoint = quinn::Endpoint::server(server_config, config.listen)?;
    info!("Listening on {}", endpoint.local_addr()?);

    Ok(endpoint)
}

async fn handle_connection(
    conn: quinn::Incoming,
    state: Arc<ServerState>,
    config: Arc<BrokerConfig>,
) -> Result<()> {
    let connection = conn.await?;
    let span = info_span!("connection", remote = %connection.remote_address());

    async {
        info!("Established");

        loop {
            let stream = connection.accept_bi().await;
            let stream = match stream {
                Err(quinn::ConnectionError::ApplicationClosed { .. }) => {
                    info!("Closed");
                    return Ok(());
                }
                Err(e) => {
                    return Err(e);
                }
                Ok(s) => s,
            };
            let fut = handle_stream(stream, state.clone(), config.clone());
            tokio::spawn(
                async move {
                    if let Err(e) = fut.await {
                        error!("Failed: {:?}", e);
                    }
                }
                .instrument(info_span!("stream")),
            );
        }
    }
    .instrument(span)
    .await?;

    Ok(())
}

async fn handle_stream(
    (send, recv): (SendStream, RecvStream),
    state: Arc<ServerState>,
    config: Arc<BrokerConfig>,
) -> Result<()> {
    let transport = QuicTransport::new(send, recv);
    let mut protocol = MqttProtocol::new(transport);

    let packet = protocol.recv_packet().await?;
    let client = PacketHandler::process_first_packet(packet, &config, &state).await?;

    if let Some((client_id, username, sender, mut receiver)) = client {
        loop {
            if let Ok(packet) = receiver.try_recv() {
                protocol.send_packet(packet).await?;
            }

            match timeout(Duration::from_millis(100), protocol.recv_packet()).await {
                Ok(recv) => match recv {
                    Ok(packet) => {
                        let should_close = PacketHandler::process_packet(
                            packet, &config, &state, &client_id, &username, &sender,
                        )
                        .await?;

                        if should_close {
                            protocol.close_connection().await?;
                            break;
                        }
                    }
                    Err(e) => {
                        error!("Error handling packet: {:?}", e);
                        break;
                    }
                },
                Err(_) => continue,
            }
        }
    }

    Ok(())
}
