use anyhow::{Context, Result};
use clap::Parser;
use mqttbytes::v5::{Disconnect, DisconnectReasonCode, Packet};
use mqttbytes::QoS;
use quinn::crypto::rustls::QuicServerConfig;
use quinn::{Endpoint, RecvStream, SendStream};
use rustls::pki_types::PrivatePkcs8KeyDer;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use shared::mqtt::{ClientID, ProtocolError};
use shared::{mqtt::MqttProtocol, transport::QuicTransport, transport::ALPN_QUIC_MQTT};
use std::fs;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::time::timeout;
use tracing::{error, info, info_span, Instrument};

use crate::handler::{BrokerConfig, PacketHandler};
use crate::state::ServerState;

#[derive(Parser, Debug)]
#[clap(name = "server-config")]
pub struct ServerConfig {
    /// TLS private key in PEM format
    #[clap(short = 'k', long = "key", requires = "cert")]
    pub key: PathBuf,
    /// TLS certificate in DER format
    #[clap(short = 'c', long = "cert", requires = "key")]
    pub cert: PathBuf,
    /// Address to listen on
    #[clap(long = "listen", default_value = "[::1]:14567")]
    pub listen: SocketAddr,
    /// Maximum number of concurrent connections to allow
    #[clap(long = "connection-limit", default_value = "10")]
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
    server_crypto.alpn_protocols = ALPN_QUIC_MQTT.iter().map(|&x| x.into()).collect();

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

    async {
        info!("Established");

        loop {
            let stream = connection.accept_bi().await;
            let stream = match stream {
                Err(
                    quinn::ConnectionError::ApplicationClosed { .. }
                    | quinn::ConnectionError::LocallyClosed { .. },
                ) => {
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
    .instrument(info_span!("connection", remote = %connection.remote_address()))
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

    let packet = match protocol.recv_packet().await {
        Ok(packet) => packet,
        Err(e) => {
            error!("Error receiving first packet: {}; closing connection", e);
            protocol.close_connection().await?;
            return Ok(());
        }
    };

    let (client_id, username, sender, mut receiver) =
        match PacketHandler::process_first_packet(packet, &config, &state).await {
            Ok(client) => client,
            Err(e) => {
                error!("Error with first packet: {}; closing connection", e);
                protocol.close_connection().await?;
                return Ok(());
            }
        };

    match handle_client(
        &client_id,
        &username,
        &state,
        &mut protocol,
        &config,
        &sender,
        &mut receiver,
    )
    .instrument(info_span!("client", client_id = %client_id))
    .await
    {
        Ok(_) => {}
        Err(e) => {
            error!("Error: {}", e);
        }
    };

    state.remove_client(&client_id).await;
    protocol.close_connection().await?;

    Ok(())
}

async fn handle_client(
    client_id: &ClientID,
    username: &str,
    state: &Arc<ServerState>,
    protocol: &mut MqttProtocol<QuicTransport>,
    config: &Arc<BrokerConfig>,
    sender: &Sender<Packet>,
    receiver: &mut Receiver<Packet>,
) -> Result<()> {
    loop {
        while let Ok(packet) = receiver.try_recv() {
            info!("Sending: {:?}", packet);
            protocol
                .send_packet(packet)
                .await
                .context("Failed to send packet")?;
        }

        if let Ok(recv) = timeout(Duration::from_millis(100), protocol.recv_packet()).await { match recv {
            Ok(packet) => {
                PacketHandler::process_packet(
                    packet, config, state, client_id, username, sender,
                )
                .await?;
            }
            Err(ProtocolError::MqttError(e)) => {
                error!("Error: {}; disconnecting client", e);
                state.remove_client(client_id).await;
                let _ = sender.try_send(mqttbytes::v5::Packet::Disconnect(Disconnect {
                    reason_code: DisconnectReasonCode::ProtocolError,
                    properties: None,
                }));
            }
            Err(e) => {
                return Err(anyhow::anyhow!(e));
            }
        } }
    }
}
