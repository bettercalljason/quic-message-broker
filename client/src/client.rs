use std::time::Duration;
use std::{fs, net::SocketAddr, path::PathBuf, sync::Arc};

use anyhow::{anyhow, Result};
use clap::Parser;
use inquire::{Select, Text};
use mqttbytes::{v5::*, QoS};
use quinn::Endpoint;
use quinn_proto::crypto::rustls::QuicClientConfig;
use rustls::pki_types::CertificateDer;
use shared::{
    mqtt::ClientID, mqtt::MqttProtocol, transport::quic::ALPN_QUIC_MQTT, transport::QuicTransport,
};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::{self, timeout};
use tokio_util::sync::CancellationToken;
use tracing::{error, info};

#[derive(Parser, Debug)]
#[clap(name = "client-config")]
pub struct ClientConfig {
    /// Remote address
    #[clap(default_value = "[::1]:14567")]
    pub remote: SocketAddr,

    /// Override hostname used for certificate verification
    #[clap(long = "host", default_value = "localhost")]
    pub host: String,

    /// Custom certificate authority to trust, in DER format
    #[clap(long = "ca")]
    pub ca: PathBuf,

    /// Address to bind on
    #[clap(long = "bind", default_value = "[::]:0")]
    pub bind: SocketAddr,

    /// Logfile to write to
    #[clap(long = "logfile", default_value = "app.log")]
    pub log_file: PathBuf,
}

#[derive(Debug)]
enum PacketOpts {
    Connect,
    Disconnect,
    Publish,
    Subscribe,
    Unsubscribe,
}

impl std::fmt::Display for PacketOpts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

pub async fn run_client(config: ClientConfig) -> Result<()> {
    let endpoint = setup_quic(&config).await?;

    info!("Connecting to {}", config.remote);
    let conn = endpoint
        .connect(config.remote, &config.host.clone())?
        .await
        .map_err(|e| anyhow!("Failed to connect: {}", e))?;
    let (send, recv) = conn
        .open_bi()
        .await
        .map_err(|e| anyhow!("Failed to open stream: {}", e))?;
    info!("Connection established");

    let transport = QuicTransport::new(send, recv);
    let mut protocol = MqttProtocol::new(transport);

    let (sender, mut receiver) = mpsc::channel(100);

    let token = CancellationToken::new();

    let cloned_token = token.clone();

    let sender_clone = sender.clone();

    let user_prompt_task = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = cloned_token.cancelled() => break,
                r = prompt_user_action(&sender_clone) => {
                    if let Err(e) = r {
                        error!("Error: {}", e);
                        break;
                    }
                }
            }
        }
    });

    let mut ping_task: Option<JoinHandle<()>> = None;

    loop {
        if user_prompt_task.is_finished() {
            info!("Exiting...");
            if let Some(handle) = ping_task {
                handle.abort();
            }
            break;
        }

        if let Ok(packet) = receiver.try_recv() {
            protocol.send_packet(packet).await?;
        }

        match timeout(Duration::from_millis(500), protocol.recv_packet()).await {
            Ok(recv) => match recv {
                Ok(Packet::ConnAck(conn_ack)) => {
                    info!("Received: {:?}", conn_ack);
                    let ping_timeout = conn_ack.properties.and_then(|p| p.server_keep_alive);
                    let sender = sender.clone();
                    if let Some(p) = ping_timeout {
                        if ping_task.is_none() {
                            let cloned_token = token.clone();

                            ping_task = Some(tokio::spawn(async move {
                                let mut interval = time::interval(Duration::from_secs(p.into()));

                                loop {
                                    tokio::select! {
                                        _ = interval.tick() => {
                                            if let Err(e) = sender.send(Packet::PingReq).await {
                                                error!("Sending PingReq failed: {}. No more PingReq's will be sent", e);
                                                break;
                                            }
                                        }
                                        _ = cloned_token.cancelled() => break,
                                    }
                                }
                            }));
                        }
                    }
                }
                Ok(packet) => {
                    info!("Received: {:?}", packet);
                }
                Err(e) => return Err(anyhow::anyhow!(e)),
            },
            Err(_) => continue,
        }
    }

    //send.finish()?;
    conn.close(0u32.into(), b"done");

    // Give the server a fair chance to receive the close packet
    endpoint.wait_idle().await;

    Ok(())
}

async fn setup_quic(config: &ClientConfig) -> Result<Endpoint> {
    let mut roots = rustls::RootCertStore::empty();
    roots.add(CertificateDer::from(fs::read(&config.ca)?))?;

    let mut client_crypto = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();

    client_crypto.alpn_protocols = ALPN_QUIC_MQTT.iter().map(|&x| x.into()).collect();

    let client_config =
        quinn::ClientConfig::new(Arc::new(QuicClientConfig::try_from(client_crypto)?));
    let mut endpoint = quinn::Endpoint::client(config.bind)?;
    endpoint.set_default_client_config(client_config);

    Ok(endpoint)
}

async fn prompt_user_action(sender: &mpsc::Sender<Packet>) -> Result<()> {
    let options = vec![
        PacketOpts::Connect,
        PacketOpts::Disconnect,
        PacketOpts::Publish,
        PacketOpts::Subscribe,
        PacketOpts::Unsubscribe,
    ];
    let ans = Select::new("Which MQTT packet do you want to send?", options).prompt()?;

    let packet = match ans {
        PacketOpts::Connect => {
            let username = Text::new("Username:").with_default("jason").prompt()?;
            let password = Text::new("Password:")
                .with_default("supersecure")
                .prompt()?;

            Ok::<Packet, anyhow::Error>(Packet::Connect(Connect {
                login: Some(Login { username, password }),
                protocol: mqttbytes::Protocol::V5,
                keep_alive: 0,
                client_id: ClientID::new().to_string(),
                clean_session: true,
                last_will: None,
                properties: None,
            }))
        }
        PacketOpts::Disconnect => Ok(Packet::Disconnect(Disconnect::new())),
        PacketOpts::Publish => {
            let topic = Text::new("Topic:").with_default("mytopic").prompt()?;
            let payload = Text::new("Payload:").with_default("Hello World").prompt()?;

            Ok(Packet::Publish(Publish::new(
                topic,
                QoS::AtMostOnce,
                payload,
            )))
        }
        PacketOpts::Subscribe => {
            let path = Text::new("Path:").with_default("mytopic").prompt()?;
            Ok(Packet::Subscribe(Subscribe::new(path, QoS::AtMostOnce)))
        }
        PacketOpts::Unsubscribe => {
            let topic = Text::new("Topic:").with_default("mytopic").prompt()?;
            Ok(Packet::Unsubscribe(Unsubscribe::new(topic)))
        }
    }?;

    sender.send(packet).await?;

    Ok(())
}
