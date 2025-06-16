use crate::transport::{Transport, TransportMessage};
use anyhow::anyhow;
use async_trait::async_trait;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio::sync::{Mutex, mpsc};

pub struct ChannelTransport {
    tx: Sender<TransportMessage>,
    rx: Mutex<Receiver<TransportMessage>>,
}

impl ChannelTransport {
    pub fn new(tx: Sender<TransportMessage>, rx: Receiver<TransportMessage>) -> Self {
        Self {
            tx,
            rx: Mutex::new(rx),
        }
    }
}

#[async_trait]
impl Transport for ChannelTransport {
    async fn send(&self, data: &TransportMessage) -> anyhow::Result<()> {
        self.tx
            .send(data.clone())
            .await
            .map_err(|e| anyhow!("send failed: {e}"))
    }

    async fn receive(&self) -> anyhow::Result<TransportMessage> {
        let mut rx = self.rx.lock().await;
        rx.recv().await.ok_or_else(|| anyhow!("channel closed"))
    }
}

pub fn channel_transport_pair(capacity: usize) -> (ChannelTransport, ChannelTransport) {
    let (tx1, rx1) = mpsc::channel::<TransportMessage>(capacity);
    let (tx2, rx2) = mpsc::channel::<TransportMessage>(capacity);

    let a = ChannelTransport::new(tx1, rx2); // side A
    let b = ChannelTransport::new(tx2, rx1); // side B

    (a, b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_channel_transport() -> anyhow::Result<()> {
        let (client, server) = channel_transport_pair(10);

        let msg = TransportMessage::Text(b"hello".to_vec());

        client.send(&msg).await?;
        let received = server.receive().await?;

        assert_eq!(received, msg);
        Ok(())
    }
}
