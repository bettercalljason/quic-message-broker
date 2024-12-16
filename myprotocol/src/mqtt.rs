use std::sync::Arc;

use async_trait::async_trait;
use bytes::BytesMut;
use mqttbytes::v5::{ConnAck, ConnAckProperties, Connect, ConnectReturnCode, Packet};
use quinn::SendStream;
use tracing::info;

use crate::{error::ServerError, protocol::ProtocolHandler, state::ServerState};

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
        send_stream: &mut SendStream,
    ) -> Result<(), ServerError> {
        info!("Packet received yay: {:?}", packet);
        match packet {
            Packet::Connect(c) => {
                server_state.add_client(&c.client_id);
                self.send_connack(send_stream).await?;
            }
            Packet::Publish(p) => {
                server_state.handle_publish(&p).await?;
            }
            _ => {}
        }
        Ok(())
    }

    async fn send_connack(&self, send_stream: &mut SendStream) -> Result<(), ServerError> {
        // Create and write a ConnAck packet

        let x = ConnAck {
            session_present: false,
            code: ConnectReturnCode::Success,
            properties: Some(ConnAckProperties::new())
        };
        let mut buf = BytesMut::new();
        x.write(&mut buf).map_err(|e|ServerError::MqttError((e)))?;

        info!("Sending: {:?}, buf: {:?}", x, buf);

        send_stream.write_all(&buf).await;
        send_stream.finish().unwrap();

        Ok(())
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
        send_stream: &mut SendStream,
    ) -> Result<(), ServerError> {
        while let Some(packet) = self.try_parse_packet(buf)? {
            self.handle_packet(packet, server_state, send_stream)
                .await?;
        }
        Ok(())
    }
}
