use std::{fs, net::SocketAddr, path::PathBuf, sync::Arc, time::Instant};

use anyhow::Context;
use anyhow::{anyhow, Result};
use bytes::BytesMut;
use clap::Parser;
use inquire::Select;
use myprotocol::{ClientID, MqttHandler, ServerError, ALPN_QUIC_HTTP};
use quinn::Endpoint;
use quinn_proto::crypto::rustls::QuicClientConfig;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio::io::AsyncReadExt;
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

impl ClientPacket {
    pub fn write(&self, buffer: &mut BytesMut) -> Result<usize, mqttbytes::Error> {
        match self {
            ClientPacket::Connect(p) => p.write(buffer),
            ClientPacket::Disconnect(p) => p.write(buffer),
        }
    }
}

#[derive(Debug)]
enum ClientPacket {
    Connect(mqttbytes::v5::Connect),
    Disconnect(mqttbytes::v5::Disconnect),
}

pub async fn run_client(config: ClientConfig) -> Result<()> {
    let endpoint = setup_quic(&config).await?;

    let start = Instant::now();
    let rebind = config.rebind;

    error!("connecting to {}", config.remote);
    let conn = endpoint
        .connect(config.remote, &config.host.clone())?
        .await
        .map_err(|e| anyhow!("failed to connect: {}", e))?;
    error!("connected at {:?}", start.elapsed());
    let (mut send, mut recv) = conn
        .open_bi()
        .await
        .map_err(|e| anyhow!("failed to open stream: {}", e))?;
    if rebind {
        let socket = std::net::UdpSocket::bind("[::]:0").unwrap();
        let addr = socket.local_addr().unwrap();
        error!("rebinding to {addr}");
        endpoint.rebind(socket).expect("rebind failed");
    }

    let mqtt_handler = MqttHandler::new(1024 * 1024);

    loop {
        let options = vec!["CONNECT", "DISCONNECT", "PUBLISH", "SUBSCRIBE", "Abort"];

        let ans = Select::new("Which MQTT packet do you want to send?", options).prompt()?;

        let res = match ans {
            "CONNECT" => ClientPacket::Connect(mqttbytes::v5::Connect::new(ClientID::new())),
            "DISCONNECT" => ClientPacket::Disconnect(mqttbytes::v5::Disconnect::new()),
            "Abort" => break,
            otherwise => {
                error!("Unhandled option {otherwise}");
                continue;
            }
        };

        info!("Sending {:?}", res);

        let mut buf = BytesMut::new();
        res.write(&mut buf)
            .map_err(|e| anyhow!("failed to send request: {}", e))?;

        send.write_all(&buf).await?;

        let mut tmp = BytesMut::new();

        let resp = recv
            .read_buf(&mut tmp)
            .await
            .map_err(|e| anyhow!("failed to read response: {}", e))?;

        if resp == 0 {
            continue; // EOF
        }

        while let Some(packet) = match mqtt_handler.read_patched(&mut tmp, 1024 * 1024) {
            Ok(packet) => Ok(Some(packet)),
            Err(mqttbytes::Error::InsufficientBytes(_)) => Ok(None),
            Err(e) => Err(ServerError::MqttError(e)),
        }? {
            info!("Received packet: {:?}", packet);
        }
    }

    send.finish()?;
    conn.close(0u32.into(), b"done");

    // Give the server a fair chance to receive the close packet
    endpoint.wait_idle().await;

    Ok(())
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
