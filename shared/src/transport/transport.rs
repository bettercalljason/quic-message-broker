use anyhow::Result;
use async_trait::async_trait;

#[async_trait]
pub trait Transport {
    async fn send(&mut self, data: &[u8]) -> Result<()>;
    async fn recv(&mut self) -> Result<Vec<u8>>;
    async fn close(&mut self) -> Result<()>;
}
