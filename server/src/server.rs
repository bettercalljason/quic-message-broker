use anyhow::Result;
use myprotocol::run_quic_listener;
use myprotocol::MqttHandler;
use myprotocol::Opt;
use myprotocol::ServerState;
use std::sync::Arc;

pub async fn run_server(options: Opt) -> Result<()> {
    // Initialize your server state
    let server_state = Arc::new(ServerState::new());

    let mqtt_handler = Arc::new(MqttHandler::new(1024 * 1024));

    run_quic_listener(options, server_state, mqtt_handler).await
}
