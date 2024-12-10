use std::sync::Arc;

use bytes::BytesMut;
use quinn::SendStream;

use crate::{error::ServerError, state::ServerState};

#[async_trait::async_trait]
pub trait ProtocolHandler {
    async fn handle_bytes(
        &self,
        buf: &mut BytesMut,
        server_state: &Arc<ServerState>,
        send_stream: &mut SendStream,
    ) -> Result<(), ServerError>;

    // You could also define methods for initialization, cleanup,
    // or other protocol-specific actions as needed.
}
