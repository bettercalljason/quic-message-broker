use anyhow::Result;
use bytes::BytesMut;
use mqttbytes::v5::Packet;
use tracing::info;

use crate::transport::Transport;

use super::MqttCodec;

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

    pub async fn send_packet(&mut self, packet: Packet) -> Result<()> {
        info!("Sending packet {:?}", packet);
        let encoded = self.codec.encode(&packet)?;
        self.transport.send(&encoded).await
    }

    pub async fn recv_packet(&mut self) -> Result<Packet> {
        loop {
            if let Some(packet) = self.codec.decode(&mut self.buffer)? {
                info!("Received packet {:?}", packet);
                return Ok(packet);
            }

            let chunk = self.transport.recv().await?;
            self.buffer.extend_from_slice(&chunk);
        }
    }

    pub async fn close_connection(&mut self) -> Result<()> {
        info!("Closing connection");
        self.transport.close().await
    }
}
