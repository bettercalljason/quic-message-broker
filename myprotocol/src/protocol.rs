use std::sync::Arc;

use bytes::BytesMut;
use quinn::SendStream;

use crate::{error::ServerError, mqtt::MqttEvent, state::ServerState};

#[async_trait::async_trait]
pub trait ProtocolHandler {
    async fn handle_bytes(
        &self,
        buf: &mut BytesMut,
        server_state: &Arc<ServerState>,
    ) -> Result<Vec<MqttEvent>, ServerError>;

    // You could also define methods for initialization, cleanup,
    // or other protocol-specific actions as needed.
}
