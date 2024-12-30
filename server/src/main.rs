use anyhow::Result;
use clap::Parser;
use server::{run_server, ServerConfig};
use tracing::error;
mod server;

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

    let config = ServerConfig::parse();
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
async fn run(config: ServerConfig) -> Result<()> {
    run_server(config).await?;

    Ok(())
}
