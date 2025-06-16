use async_trait::async_trait;

pub mod channel;
pub mod websocket;

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum TransportMessage {
    Text(Vec<u8>),
    Binary(Vec<u8>),
}

#[async_trait]
pub trait Transport: Send + Sync + 'static {
    async fn send(&self, msg: &TransportMessage) -> anyhow::Result<()>;
    async fn receive(&self) -> anyhow::Result<TransportMessage>;
    async fn close(&self) -> anyhow::Result<()> {
        Ok(())
    }
}
