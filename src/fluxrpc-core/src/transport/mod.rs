use async_trait::async_trait;

pub mod channel;
pub mod websocket;

#[async_trait]
pub trait Transport: Send + Sync + 'static {
    async fn send(&self, data: &[u8]) -> anyhow::Result<()>;
    async fn receive(&self) -> anyhow::Result<Vec<u8>>;
    async fn close(&self) -> anyhow::Result<()> {
        Ok(())
    }
}
