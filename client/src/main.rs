use std::net::{IpAddr, Ipv6Addr, SocketAddr};

use anyhow::Result;
use clap::Parser;
use client::{run_client, ClientConfig};
use inquire::CustomType;
use tracing::error;

mod client;

fn main() {
    tracing::subscriber::set_global_default(
        tracing_subscriber::FmtSubscriber::builder()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or(tracing_subscriber::EnvFilter::new("info")),
            )
            .finish(),
    )
    .unwrap();

    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install rustls crypto provider");

    let config = ClientConfig::parse();
    let code = {
        if let Err(e) = run(config) {
            error!("ERROR: {e}");
            1
        } else {
            0
        }
    };
    ::std::process::exit(code);
}

#[tokio::main]
async fn run(config: ClientConfig) -> Result<()> {
    let remote = CustomType::<SocketAddr>::new("Enter broker address:")
        .with_default(SocketAddr::new(
            IpAddr::V6(Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 1)),
            4433,
        ))
        .prompt()
        .unwrap();

    let my_config = ClientConfig { remote, ..config };

    run_client(my_config).await?;

    Ok(())
}
