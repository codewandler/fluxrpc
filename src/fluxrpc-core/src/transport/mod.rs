use async_trait::async_trait;
use std::fmt::Debug;

pub mod channel;
pub mod websocket;

#[derive(Clone, Eq, PartialEq)]
pub enum TransportMessage {
    Text(Vec<u8>),
    Binary(Vec<u8>),
}

impl Debug for TransportMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TransportMessage::Text(bytes) => write!(
                f,
                "TransportMessage::Text({:?})",
                String::from_utf8_lossy(bytes)
            ),
            TransportMessage::Binary(bytes) => {
                write!(f, "TransportMessage::Binary(len={:?})", bytes.len())
            }
        }
    }
}

#[async_trait]
pub trait Transport: Send + Sync + 'static {
    async fn send(&self, msg: &TransportMessage) -> anyhow::Result<()>;
    async fn receive(&self) -> anyhow::Result<TransportMessage>;
    async fn close(&self) -> anyhow::Result<()> {
        Ok(())
    }
}
