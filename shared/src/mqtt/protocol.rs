use anyhow::Result;
use bytes::BytesMut;
use mqttbytes::v5::Packet;
use std::{error::Error, fmt::Debug};
use tracing::trace;

use crate::transport::Transport;

use super::{MqttCodec, MqttError};

#[derive(thiserror::Error)]
pub enum ProtocolError {
    #[error("MQTT: {0}")]
    MqttError(#[from] MqttError),

    #[error("Transport: {0}")]
    TransportError(#[from] anyhow::Error),
}

impl Debug for ProtocolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "{}", self)?;
        if let Some(source) = self.source() {
            writeln!(f, "Caused by:\n\t{}", source)?;
        }
        Ok(())
    }
}

pub struct MqttProtocol<T: Transport> {
    transport: T,
    buffer: BytesMut,
    codec: MqttCodec,
}

impl<T: Transport> MqttProtocol<T> {
    pub fn new(transport: T) -> Self {
        Self {
            transport,
            buffer: BytesMut::with_capacity(4096),
            codec: MqttCodec {
                max_packet_size: 1024 * 1024,
            },
        }
    }

    pub async fn send_packet(&mut self, packet: Packet) -> Result<(), ProtocolError> {
        trace!("Sending packet {:?}", packet);
        let encoded = self
            .codec
            .encode(&packet)
            .map_err(|e| ProtocolError::MqttError(e))?;
        self.transport
            .send(&encoded)
            .await
            .map_err(ProtocolError::TransportError)
    }

    pub async fn recv_packet(&mut self) -> Result<Packet, ProtocolError> {
        loop {
            if let Some(packet) = self
                .codec
                .decode(&mut self.buffer)
                .map_err(|e| ProtocolError::MqttError(e))?
            {
                trace!("Received packet {:?}", packet);
                return Ok(packet);
            }

            let chunk = self
                .transport
                .recv()
                .await
                .map_err(ProtocolError::TransportError)?;
            self.buffer.extend_from_slice(&chunk);
        }
    }

    pub async fn close_connection(&mut self) -> Result<(), ProtocolError> {
        trace!("Closing connection");
        self.transport
            .close()
            .await
            .map_err(ProtocolError::TransportError)
    }
}
