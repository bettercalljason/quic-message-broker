use std::{
    fs,
    io::{self, Write},
    net::SocketAddr,
    path::PathBuf,
    sync::Arc,
    time::Instant,
};

use anyhow::Context;
use anyhow::{anyhow, Result};
use bytes::BytesMut;
use clap::Parser;
use myprotocol::{ServerError, ALPN_QUIC_HTTP};
use quinn::Endpoint;
use quinn_proto::crypto::rustls::QuicClientConfig;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio::io::AsyncReadExt;
use tracing::info;

#[derive(Parser, Debug)]
#[clap(name = "client-config")]
pub struct ClientConfig {
    /// Perform NSS-compatible TLS key logging to the file specified in `SSLKEYLOGFILE`.
    #[clap(long = "keylog")]
    keylog: bool,

    #[clap(default_value = "[::1]:4433")]
    remote: SocketAddr,

    /// Override hostname used for certificate verification
    #[clap(long = "host", default_value = "localhost")]
    host: String,

    /// Custom certificate authority to trust, in DER format
    #[clap(long = "ca", default_value = "..\\tlsgen\\server\\cert.der")]
    ca: PathBuf,

    /// Simulate NAT rebinding after connecting
    #[clap(long = "rebind")]
    rebind: bool,

    /// Address to bind on
    #[clap(long = "bind", default_value = "[::]:0")]
    bind: SocketAddr,

    /// TLS private key in PEM format
    #[clap(
        short = 'k',
        long = "key",
        requires = "cert",
        default_value = "..\\tlsgen\\client\\key.der"
    )]
    pub key: PathBuf,
    /// TLS certificate in PEM format
    #[clap(
        short = 'c',
        long = "cert",
        requires = "key",
        default_value = "..\\tlsgen\\client\\cert.der"
    )]
    pub cert: PathBuf,
}

pub async fn run_client(config: ClientConfig) -> Result<()> {
    let endpoint = setup_quic(&config).await?;

    let start = Instant::now();
    let rebind = config.rebind;

    eprintln!("connecting to {}", config.remote);
    let conn = endpoint
        .connect(config.remote, &config.host.clone())?
        .await
        .map_err(|e| anyhow!("failed to connect: {}", e))?;
    eprintln!("connected at {:?}", start.elapsed());
    let (mut send, mut recv) = conn
        .open_bi()
        .await
        .map_err(|e| anyhow!("failed to open stream: {}", e))?;
    if rebind {
        let socket = std::net::UdpSocket::bind("[::]:0").unwrap();
        let addr = socket.local_addr().unwrap();
        eprintln!("rebinding to {addr}");
        endpoint.rebind(socket).expect("rebind failed");
    }

    let connect_packet = mqttbytes::v5::Connect::new("bla");

    let mut buf = BytesMut::new();
    connect_packet
        .write(&mut buf)
        .map_err(|e| anyhow!("failed to send request: {}", e))?;

    send.write_all(&buf).await?;

    //send.finish().unwrap();
    let response_start = Instant::now();
    eprintln!("request sent at {:?}", response_start - start);

    let mut tmp = BytesMut::new();

    loop {
        let resp = recv
            .read_buf(&mut tmp)
            .await
            .map_err(|e| anyhow!("failed to read response: {}", e))?;

        if resp == 0 {
            break; // EOF
        }

        while let Some(packet) = match mqttbytes::v5::read(&mut tmp, 1024 * 1024) {
            Ok(packet) => Ok(Some(packet)),
            Err(mqttbytes::Error::InsufficientBytes(_)) => Ok(None),
            Err(e) => Err(ServerError::MqttError(e)),
        }? {
            info!("Received packet: {:?}", packet);
        }

        let disconnect_packet = mqttbytes::v5::Disconnect::new();
        disconnect_packet
            .write(&mut buf)
            .map_err(|e| anyhow!("failed to send request: {}", e))?;

        info!("Sent {:?}", disconnect_packet);

        send.write_all(&buf).await?;
    }

    io::stdout().flush().unwrap();
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
