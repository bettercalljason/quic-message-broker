use anyhow::Result;
use clap::Parser;
use mqttbytes::v5::{ConnAck, ConnAckProperties, ConnectReturnCode, Publish};
use myprotocol::{ClientID, MqttEvent, MqttHandler, OutgoingMessage, ALPN_QUIC_HTTP};
use rustls::server::WebPkiClientVerifier;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::{fs, sync::Arc};
use tokio::sync::mpsc;
use tracing::{error, info};

use anyhow::Context;
use bytes::BytesMut;
use quinn::crypto::rustls::QuicServerConfig;
use quinn::{Connection, Endpoint, SendStream};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio::io::AsyncReadExt;

use myprotocol::{ProtocolHandler, ServerError, ServerState};

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
    // Initialize your server state
    let server_state = Arc::new(ServerState::new());

    let mqtt_handler = Arc::new(MqttHandler::new(1024 * 1024));

    let endpoint = setup_quic(config).await?;
    accept_incoming(&endpoint, server_state, mqtt_handler).await?;
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
                .unwrap_or_else(|e| error!("ERROR: {e}"));
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

        // Expecting a CONNECT packet here...
        // Read data from recv_stream
        let n = recv_stream.read_buf(&mut buf).await.unwrap_or(0);

        if n == 0 {
            // Stream closed (EOF)
            break;
        }

        let mut a_client_id: Option<ClientID> = None;

        let mut events = handler.handle_bytes(&mut buf, &server_state).await?;

        if let Some(pos) = events
            .iter()
            .position(|e| matches!(e, MqttEvent::ClientConnected { .. }))
        {
            // Remove the event from the vector
            let event = events.remove(pos);

            // Now handle the ClientConnected event seperatley
            if let MqttEvent::ClientConnected { client_id } = event {
                let (tx, rx) = mpsc::channel(100);

                server_state.add_client(&client_id, tx).await?;
                a_client_id = Some(client_id.clone());

                // Now you have full control over how you handle send_stream
                let send_stream_for_task = send_stream;

                tokio::spawn(async move {
                    if let Err(e) = connection_task(send_stream_for_task, rx, client_id).await {
                        error!("Connection task ended with error: {:?}", e);
                    }
                });
            }
        };

        if let Some(client_id) = a_client_id {
            loop {
                // Handle the rest of the events
                for event in events {
                    info!("event: {:?}", event);
                    match event {
                        MqttEvent::ClientConnected { .. } => {
                            server_state.second_connect_error(&client_id).await;
                            server_state.remove_client(&client_id).await;
                        }
                        MqttEvent::ClientDisconnected => {
                            info!("Client {} disconnected", client_id);
                            server_state.remove_client(&client_id).await;
                        }
                        MqttEvent::ClientSubscribed { topic, qos } => {
                            server_state.add_subscription(&client_id, &topic, qos).await;
                        }
                        MqttEvent::PublishReceived { topic, payload } => {
                            server_state
                                .handle_publish(&Publish::new(
                                    topic,
                                    mqttbytes::QoS::AtLeastOnce,
                                    payload,
                                ))
                                .await?;
                        }
                    }
                }
                let n = recv_stream.read_buf(&mut buf).await.unwrap_or(0);
                events = handler.handle_bytes(&mut buf, &server_state).await?;
            }
        } else {
            panic!("No client ID")
        }
    }
    Ok(())
}

async fn connection_task(
    mut send_stream: SendStream,
    mut receiver: mpsc::Receiver<OutgoingMessage>,
    client_id: ClientID,
) -> Result<(), ServerError> {
    // This task runs per client connection.
    // It listens for OutgoingMessage from the receiver.
    while let Some(msg) = receiver.recv().await {
        let mut buf = BytesMut::new();
        match msg {
            OutgoingMessage::ConnAck(packet) => {
                packet
                    .write(&mut buf)
                    .map_err(|e| ServerError::MqttError(e))?;

                if packet.code == ConnectReturnCode::ProtocolError {
                    break;
                }
            }
            OutgoingMessage::Publish(packet) => {
                packet
                    .write(&mut buf)
                    .map_err(|e| ServerError::MqttError(e))?;
            }
            OutgoingMessage::PubAck(packet) => {
                packet
                    .write(&mut buf)
                    .map_err(|e| ServerError::MqttError(e))?;
            }
        }

        // Write the serialized packet to the QUIC stream
        send_stream
            .write_all(&buf)
            .await
            .map_err(|e| ServerError::QuinnWriteError(e))?;
    }

    // When the sender side is dropped (or client is removed), this loop ends.
    // We can close the connection gracefully here if needed.
    send_stream
        .finish()
        .map_err(|e| ServerError::QuinnClosedStreamError(e))?;
    info!("Connection task for {} closed", client_id);

    Ok(())
}
