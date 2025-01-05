use anyhow::Result;
use clap::Parser;
use client::{run_client, ClientConfig};
use tokio::signal;
use tokio_util::sync::CancellationToken;
use tracing::error;

mod client;

fn main() {
    let config = ClientConfig::parse();

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
    let my_config = ClientConfig { ..config };

    let token = CancellationToken::new();
    let token_clone = token.clone();

    let task_handle = tokio::spawn(async move {
        if let Err(err) = run_client(my_config, token_clone).await {
            error!("Task failed: {}", err);
        }
    });

    match signal::ctrl_c().await {
        Ok(()) => {}
        Err(err) => {
            error!("Unable to listen for shutdown signal: {}", err);
        }
    }

    token.cancel();

    task_handle.await.unwrap();

    Ok(())
}
