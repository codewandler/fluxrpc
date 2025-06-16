use crate::codec::Codec;
use crate::session::RpcSessionHandler;
use crate::transport::{Transport, TransportMessage};
use crate::{RpcSession, SessionState};
use anyhow::anyhow;
use async_trait::async_trait;
use std::sync::Arc;
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

pub fn new_channel_sessions<C, S>(
    c: C,
    h: Arc<dyn RpcSessionHandler<State = S>>,
    s1: S,
    s2: S,
) -> (
    Arc<RpcSession<C, ChannelTransport, S>>,
    Arc<RpcSession<C, ChannelTransport, S>>,
)
where
    C: Codec,
    S: SessionState,
{
    let (t1, t2) = channel_transport_pair(10);
    let s1 = RpcSession::create(t1, c.clone(), h.clone(), s1);
    let s2 = RpcSession::create(t2, c, h.clone(), s2);
    (s1, s2)
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
