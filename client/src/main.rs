use std::{
    fs::File,
    net::{IpAddr, Ipv6Addr, SocketAddr},
    sync::Arc,
};

use anyhow::Result;
use clap::Parser;
use client::{run_client, ClientConfig};
use inquire::CustomType;
use tracing::error;

mod client;

fn main() {
    let config = ClientConfig::parse();

    // Open or create a log file
    let file = File::create(&config.log_file).expect("Failed to create log file");
    let file = Arc::new(file); // Arc<Mutex> for safe access across threads

    tracing::subscriber::set_global_default(
        tracing_subscriber::FmtSubscriber::builder()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or(tracing_subscriber::EnvFilter::new("info")),
            )
            .with_writer(file)
            .finish(),
    )
    .unwrap();

    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install rustls crypto provider");

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
            8883,
        ))
        .prompt()?;

    let my_config = ClientConfig { remote, ..config };

    run_client(my_config).await?;

    Ok(())
}
