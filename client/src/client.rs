use std::{
    fs,
    io::{self, Write},
    net::SocketAddr,
    path::PathBuf,
    sync::Arc,
    time::Instant,
};

use anyhow::{anyhow, Result};
use bytes::BytesMut;
use clap::Parser;
use myprotocol::{ServerError, ALPN_QUIC_HTTP};
use quinn::Endpoint;
use quinn_proto::crypto::rustls::QuicClientConfig;
use rustls::pki_types::CertificateDer;
use tokio::io::AsyncReadExt;
use tracing::{error, info};

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
    #[clap(long = "ca")]
    ca: Option<PathBuf>,

    /// Simulate NAT rebinding after connecting
    #[clap(long = "rebind")]
    rebind: bool,

    /// Address to bind on
    #[clap(long = "bind", default_value = "[::]:0")]
    bind: SocketAddr,
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

    //   send.write_all(request.as_bytes())
    //       .await
    //       .map_err(|e| anyhow!("failed to send request: {}", e))?;
    send.finish().unwrap();
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
    }

    io::stdout().flush().unwrap();
    conn.close(0u32.into(), b"done");

    // Give the server a fair chance to receive the close packet
    endpoint.wait_idle().await;

    Ok(())
}

async fn setup_quic(config: &ClientConfig) -> Result<Endpoint> {
    let mut roots = rustls::RootCertStore::empty();
    if let Some(ca_path) = &config.ca {
        roots.add(CertificateDer::from(fs::read(ca_path)?))?;
    } else {
        let dirs = directories_next::ProjectDirs::from("org", "quinn", "quinn-examples").unwrap();
        match fs::read(dirs.data_local_dir().join("cert.der")) {
            Ok(cert) => {
                roots.add(CertificateDer::from(cert))?;
            }
            Err(ref e) if e.kind() == io::ErrorKind::NotFound => {
                info!("local server certificate not found");
            }
            Err(e) => {
                error!("failed to open local server certificate: {}", e);
            }
        }
    }
    let mut client_crypto = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();

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

