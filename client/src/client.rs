use core::fmt;
use std::{fs, net::SocketAddr, path::PathBuf, sync::Arc, time::Instant};

use anyhow::Context;
use anyhow::{anyhow, Result};
use clap::Parser;
use inquire::{Select, Text};
use mqttbytes::{v5::*, QoS};
use myprotocol::{ClientID, MqttProtocol, QuicTransport, ALPN_QUIC_HTTP};
use quinn::Endpoint;
use quinn_proto::crypto::rustls::QuicClientConfig;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tracing::{error, info};

#[derive(Parser, Debug)]
#[clap(name = "client-config")]
pub struct ClientConfig {
    /// Perform NSS-compatible TLS key logging to the file specified in `SSLKEYLOGFILE`.
    #[clap(long = "keylog")]
    pub keylog: bool,

    #[clap(default_value = "[::1]:4433")]
    pub remote: SocketAddr,

    /// Override hostname used for certificate verification
    #[clap(long = "host", default_value = "localhost")]
    pub host: String,

    /// Custom certificate authority to trust, in DER format
    #[clap(
        long = "ca",
        default_value = "C:\\GitHub\\quic-message-broker\\tlsgen\\server\\cert.der"
    )]
    pub ca: PathBuf,

    /// Simulate NAT rebinding after connecting
    #[clap(long = "rebind")]
    pub rebind: bool,

    /// Address to bind on
    #[clap(long = "bind", default_value = "[::]:0")]
    pub bind: SocketAddr,

    /// TLS private key in PEM format
    #[clap(
        short = 'k',
        long = "key",
        requires = "cert",
        default_value = "C:\\GitHub\\quic-message-broker\\tlsgen\\client\\key.der"
    )]
    pub key: PathBuf,
    /// TLS certificate in PEM format
    #[clap(
        short = 'c',
        long = "cert",
        requires = "key",
        default_value = "C:\\GitHub\\quic-message-broker\\tlsgen\\client\\cert.der"
    )]
    pub cert: PathBuf,
}

#[derive(Debug)]
enum PacketOpts {
    Connect,
    Disconnect,
    Publish,
    Subscribe,
    Unsubscribe,
    Exit,
}

impl fmt::Display for PacketOpts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

pub async fn run_client(config: ClientConfig) -> Result<()> {
    let endpoint = setup_quic(&config).await?;

    let start = Instant::now();
    let rebind = config.rebind;

    info!("connecting to {}", config.remote);
    let conn = endpoint
        .connect(config.remote, &config.host.clone())?
        .await
        .map_err(|e| anyhow!("failed to connect: {}", e))?;
    info!("connected at {:?}", start.elapsed());
    let (send, recv) = conn
        .open_bi()
        .await
        .map_err(|e| anyhow!("failed to open stream: {}", e))?;
    if rebind {
        let socket = std::net::UdpSocket::bind("[::]:0").unwrap();
        let addr = socket.local_addr().unwrap();
        info!("rebinding to {addr}");
        endpoint.rebind(socket).expect("rebind failed");
    }

    let transport = QuicTransport::new(send, recv);
    let mut protocol = MqttProtocol::new(transport);

    loop {
        match prompt_user_action() {
            Ok(packet) => protocol.send_packet(packet).await?,
            Err(e) => {
                error!("Prompt error: {:?}", e);
                break;
            }
        }

        match protocol.recv_packet().await {
            Ok(packet) => {
                info!("Received packet: {:?}", packet);
            }
            Err(e) => {
                error!("Error handling packet: {:?}", e);
                break;
            }
        }
    }

    //send.finish()?;
    conn.close(0u32.into(), b"done");

    // Give the server a fair chance to receive the close packet
    endpoint.wait_idle().await;

    Ok(())
}

fn prompt_user_action() -> Result<Packet> {
    let options = vec![
        PacketOpts::Connect,
        PacketOpts::Disconnect,
        PacketOpts::Publish,
        PacketOpts::Subscribe,
        PacketOpts::Unsubscribe,
        PacketOpts::Exit,
    ];

    let ans = Select::new("Which MQTT packet do you want to send?", options).prompt()?;

    match ans {
        PacketOpts::Connect => Ok(Packet::Connect(Connect::new(ClientID::new()))),

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
        PacketOpts::Exit => Err(anyhow!("Abort")),
    }
}

async fn setup_quic(config: &ClientConfig) -> Result<Endpoint> {
    let mut roots = rustls::RootCertStore::empty();
    roots.add(CertificateDer::from(fs::read(&config.ca)?))?;

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

    let mut client_crypto = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        //.with_no_client_auth();
        .with_client_auth_cert(certs, key)?;

    client_crypto.alpn_protocols = ALPN_QUIC_HTTP.iter().map(|&x| x.into()).collect();
    if config.keylog {
        client_crypto.key_log = Arc::new(rustls::KeyLogFile::new());
    }

    let client_config =
        quinn::ClientConfig::new(Arc::new(QuicClientConfig::try_from(client_crypto)?));
    let mut endpoint = quinn::Endpoint::client(config.bind)?;
    endpoint.set_default_client_config(client_config);

    Ok(endpoint)
}
