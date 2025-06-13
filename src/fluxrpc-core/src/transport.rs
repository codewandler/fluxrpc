use async_trait::async_trait;

#[async_trait]
pub trait Transport: Send + Sync + 'static {
    async fn send(&self, data: &[u8]) -> anyhow::Result<()>;
    async fn receive(&self) -> anyhow::Result<Vec<u8>>;
}

pub mod channel {
    use crate::transport::Transport;
    use anyhow::anyhow;
    use async_trait::async_trait;
    use tokio::sync::mpsc::{Receiver, Sender};
    use tokio::sync::{Mutex, mpsc};

    pub struct ChannelTransport {
        tx: Sender<Vec<u8>>,
        rx: Mutex<Receiver<Vec<u8>>>,
    }

    impl ChannelTransport {
        pub fn new(tx: Sender<Vec<u8>>, rx: Receiver<Vec<u8>>) -> Self {
            Self {
                tx,
                rx: Mutex::new(rx),
            }
        }
    }

    #[async_trait]
    impl Transport for ChannelTransport {
        async fn send(&self, data: &[u8]) -> anyhow::Result<()> {
            self.tx
                .send(data.to_vec())
                .await
                .map_err(|e| anyhow!("send failed: {e}"))
        }

        async fn receive(&self) -> anyhow::Result<Vec<u8>> {
            let mut rx = self.rx.lock().await;
            rx.recv().await.ok_or_else(|| anyhow!("channel closed"))
        }
    }

    pub fn channel_transport_pair(capacity: usize) -> (ChannelTransport, ChannelTransport) {
        let (tx1, rx1) = mpsc::channel::<Vec<u8>>(capacity);
        let (tx2, rx2) = mpsc::channel::<Vec<u8>>(capacity);

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

            let msg = b"hello".to_vec();

            client.send(&msg).await?;
            let received = server.receive().await?;

            assert_eq!(received, msg);
            Ok(())
        }
    }
}
