use std::sync::Arc;

use async_trait::async_trait;
use bytes::BytesMut;
use mqttbytes::v5::{ConnAck, ConnAckProperties, Connect, ConnectReturnCode, Packet};
use quinn::SendStream;
use tokio::sync::mpsc;
use tracing::{error, info};

use crate::{error::ServerError, protocol::ProtocolHandler, state::ServerState, OutgoingMessage};

pub enum MqttEvent {
    ClientConnected { client_id: String },
    PublishReceived { topic: String, payload: Vec<u8> },
    ClientSubscribed { topic: String, qos: u8 },
    ClientDisconnected,
    // ... other events
}

pub struct MqttHandler {
    max_packet_size: usize,
}

impl MqttHandler {
    pub fn new(max_packet_size: usize) -> Self {
        MqttHandler { max_packet_size }
    }

    async fn handle_packet(
        &self,
        packet: Packet,
        server_state: &Arc<ServerState>,
    ) -> Result<MqttEvent, ServerError> {
        match packet {
            Packet::Connect(p) => Ok(MqttEvent::ClientConnected {
                client_id: p.client_id,
            }),
            Packet::Subscribe(p) => Ok(MqttEvent::ClientSubscribed {
                topic: "foo".to_string(),
                qos: 1
            }),
            _ => todo!(),
        }
    }

    fn try_parse_packet(&self, buf: &mut BytesMut) -> Result<Option<Packet>, ServerError> {
        match mqttbytes::v5::read(buf, self.max_packet_size) {
            Ok(packet) => Ok(Some(packet)),
            Err(mqttbytes::Error::InsufficientBytes(_)) => Ok(None),
            Err(e) => Err(ServerError::MqttError(e)),
        }
    }
}

#[async_trait]
impl ProtocolHandler for MqttHandler {
    async fn handle_bytes(
        &self,
        buf: &mut BytesMut,
        server_state: &Arc<ServerState>,
    ) -> Result<Vec<MqttEvent>, ServerError> {
        let mut events = Vec::new();

        while let Some(packet) = self.try_parse_packet(buf)? {
            let event = self.handle_packet(packet, server_state).await?;
            events.push(event);
        }

        Ok(events)
    }
}
