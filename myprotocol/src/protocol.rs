use std::sync::Arc;

use anyhow::Result;
use bytes::BytesMut;
use quinn::SendStream;

use crate::{mqtt::MqttEvent, state::ServerState};

#[async_trait::async_trait]
pub trait ProtocolHandler {
    async fn handle_bytes(
        &self,
        buf: &mut BytesMut,
        server_state: &Arc<ServerState>,
    ) -> Result<Vec<MqttEvent>>;

    // You could also define methods for initialization, cleanup,
    // or other protocol-specific actions as needed.
}
