use anyhow::{anyhow, Result};
use async_trait::async_trait;
use bytes::BytesMut;
use quinn::{RecvStream, SendStream};
use tokio::io::AsyncReadExt;

use super::Transport;

pub const ALPN_QUIC_MQTT: &[&[u8]] = &[b"mqtt"]; // According to https://www.iana.org/assignments/tls-extensiontype-values/tls-extensiontype-values.xhtml#alpn-protocol-ids

pub struct QuicTransport {
    send: SendStream,
    recv: RecvStream,
}

impl QuicTransport {
    pub fn new(send: SendStream, recv: RecvStream) -> Self {
        Self { send, recv }
    }
}

#[async_trait]
impl Transport for QuicTransport {
    async fn send(&mut self, data: &[u8]) -> Result<()> {
        self.send.write_all(data).await?;
        Ok(())
    }

    async fn recv(&mut self) -> Result<Vec<u8>> {
        let mut buffer = BytesMut::new();
        let n = self.recv.read_buf(&mut buffer).await?;

        if n == 0 {
            return Err(anyhow!("Stream closed"));
        }

        Ok(buffer.split_to(n).to_vec())
    }

    async fn close(&mut self) -> Result<()> {
        self.send.finish()?;
        Ok(())
    }
}
