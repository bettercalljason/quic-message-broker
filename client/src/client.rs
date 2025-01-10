use std::collections::HashSet;
use std::iter::zip;
use std::time::Duration;
use std::{fs, net::SocketAddr, path::PathBuf, sync::Arc};

use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};
use mqttbytes::{v5::*, PacketType, QoS};
use quinn::Endpoint;
use quinn_proto::crypto::rustls::QuicClientConfig;
use rustls::pki_types::CertificateDer;
use shared::mqtt::packet_type;
use shared::{mqtt::MqttProtocol, transport::quic::ALPN_QUIC_MQTT, transport::QuicTransport};
use sysinfo::System;
use tokio::time::{self};
use tokio_util::sync::CancellationToken;
use tracing::info;

#[derive(Copy, Hash, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum, Debug)]
pub enum PublishData {
    CpuUsage,
    UsedMemory,
}

#[derive(Parser, Debug)]
#[clap(name = "client-config")]
pub struct ClientConfig {
    /// Remote address
    #[clap(default_value = "[::1]:8883")]
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

    #[clap(long = "username", default_value = "jason")]
    pub username: String,

    #[clap(long = "password", default_value = "supersecure")]
    pub password: String,

    #[clap(long = "subscribe", default_values = ["system/home/#"])]
    pub subscribed_topics: Option<Vec<String>>,

    #[clap(long = "publish", default_values = ["cpu-usage", "used-memory"])]
    pub publish: Option<Vec<PublishData>>,

    #[clap(long = "ping", default_value = "true")]
    pub ping: bool,
}

pub async fn run_client(config: ClientConfig, cancel_token: CancellationToken) -> Result<()> {
    let mut sys = System::new();

    let endpoint = setup_quic(&config).await?;

    info!("Connecting to {}", config.remote);
    let conn = endpoint
        .connect(config.remote, &config.host.clone())?
        .await
        .context("Failed to connect")?;
    let (send, recv) = conn.open_bi().await.context("Failed to open stream")?;

    info!("Connection established");

    let transport = QuicTransport::new(send, recv);
    let mut protocol = MqttProtocol::new(transport);

    protocol
        .send_packet(Packet::Connect(Connect {
            protocol: mqttbytes::Protocol::V5,
            keep_alive: 0,
            client_id: "home".to_string(),
            clean_session: true,
            last_will: None,
            login: Some(Login {
                username: config.username,
                password: config.password,
            }),
            properties: None,
        }))
        .await
        .context("Failed to connect with MQTT")?;

    // Expect ConnAck
    let conn_ack = match protocol.recv_packet().await {
        Ok(Packet::ConnAck(conn_ack)) => {
            info!("Received {:?}", PacketType::ConnAck);
            conn_ack
        }
        Ok(packet) => {
            return Err(anyhow::anyhow!(
                "Expected {:?}, received: {:?}",
                PacketType::ConnAck,
                packet_type(packet)
            ))
        }
        Err(e) => {
            return Err(
                anyhow::anyhow!(e).context(format!("Failed to receive {:?}", PacketType::ConnAck))
            )
        }
    };

    sys.refresh_cpu_usage();

    let publish_period = Duration::from_secs(10);

    if let Some(subscribed_topics) = &config.subscribed_topics {
        info!("Subscribing to {:?}", subscribed_topics);
        protocol
            .send_packet(Packet::Subscribe(Subscribe {
                pkid: 0,
                filters: subscribed_topics
                    .iter()
                    .map(|topic| SubscribeFilter::new(topic.to_string(), QoS::AtMostOnce))
                    .collect(),
                properties: None,
            }))
            .await
            .context("Failed to subscribe")?;
    }

    // Expect SubAck
    match protocol.recv_packet().await {
        Ok(Packet::SubAck(sub_ack)) => {
            let subscribed_topics = config.subscribed_topics.unwrap();
            let combined = zip(subscribed_topics, sub_ack.return_codes);
            for (topic, return_code) in combined {
                info!(
                    "Subscription for '{}' acknowledged with {:?}",
                    topic, return_code
                );
            }
        }
        Ok(packet) => {
            return Err(anyhow::anyhow!(
                "Expected {:?}, received: {:?}",
                PacketType::SubAck,
                packet_type(packet)
            ))
        }
        Err(e) => {
            return Err(
                anyhow::anyhow!(e).context(format!("Failed to receive {:?}", PacketType::SubAck))
            )
        }
    }

    let publish_config: HashSet<PublishData> =
        config.publish.unwrap_or_default().into_iter().collect();

    if !publish_config.is_empty() {
        info!(
            "Publishing {:?} every {}s",
            publish_config,
            publish_period.as_secs()
        );
    }

    let mut publish_interval = time::interval(publish_period);
    let mut ping_interval = time::interval(Duration::from_secs(
        conn_ack
            .properties
            .map(|properties| properties.server_keep_alive)
            .flatten()
            .unwrap_or(10)
            .into(),
    ));

    loop {
        tokio::select! {
            _ = cancel_token.cancelled() => {
                info!("Disconnecting...");
                protocol.send_packet(Packet::Disconnect(Disconnect::new())).await?;

                conn.close(0u32.into(), b"done");

                // Give the server a fair chance to receive the close packet
                endpoint.wait_idle().await;

                return Ok(());
            }
            _ = publish_interval.tick() => {
                    for p in publish_config.iter() {
                        match p {
                            PublishData::CpuUsage => {
                                sys.refresh_cpu_usage();
                                for cpu in sys.cpus() {
                                    protocol
                                        .send_packet(Packet::Publish(Publish::new(
                                            format!("system/home/cpu/{}/usage", cpu.name()),
                                            QoS::AtMostOnce,
                                            cpu.cpu_usage().to_string(),
                                        )))
                                        .await
                                        .context("Failed to PUBLISH CPU usage")?;
                                }
                            },
                            PublishData::UsedMemory => {
                                sys.refresh_memory();
                                protocol
                                .send_packet(Packet::Publish(Publish::new(
                                    "system/home/memory/used",
                                    QoS::AtMostOnce,
                                    sys.used_memory().to_string(),
                                )))
                                .await
                                .context("Failed to PUBLISH used memory")?;
                            },
                        }
                    }
            }
            _ = ping_interval.tick() => {
                if config.ping {
                    info!("Sending {:?}", PacketType::PingReq);
                    protocol.send_packet(Packet::PingReq).await.context("Failed to send PingReq")?;
                }
            }
            packet = protocol.recv_packet() => {
                match packet {
                    Ok(Packet::Publish(publish)) => {
                        let payload = String::from_utf8(publish.payload.to_vec()).unwrap_or_else(|e| format!("Parsing error: {}", e));
                        info!("Received {:?} for topic {} with payload: {}", PacketType::Publish, publish.topic, payload);
                    }
                    Ok(packet) => {
                        info!("Received {:?}", packet);
                    }
                    Err(e) => return Err(anyhow::anyhow!(e)),
                }
            }
        }
    }
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
