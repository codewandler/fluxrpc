pub mod client {
    use crate::codec::Codec;
    use crate::session::{RpcSession, RpcSessionHandler, SessionState};
    use crate::transport::{Transport, TransportMessage};
    use async_trait::async_trait;
    use ezsockets::{Bytes, Client, Error, Utf8Bytes};
    use std::sync::Arc;
    use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
    use tokio::sync::{Mutex, oneshot};

    pub struct ClientHandler {
        tx: UnboundedSender<TransportMessage>,
        on_connected: Option<oneshot::Sender<()>>,
    }

    pub struct ClientTransport {
        rx: Mutex<UnboundedReceiver<TransportMessage>>,
        handle: Client<ClientHandler>,
    }

    #[async_trait]
    impl ezsockets::ClientExt for ClientHandler {
        type Call = ();

        async fn on_text(&mut self, text: Utf8Bytes) -> Result<(), Error> {
            self.tx
                .send(TransportMessage::Text(text.as_bytes().into()))?;
            Ok(())
        }

        async fn on_binary(&mut self, bytes: Bytes) -> Result<(), Error> {
            self.tx
                .send(TransportMessage::Binary(bytes.to_vec().into()))?;
            Ok(())
        }

        async fn on_call(&mut self, call: Self::Call) -> Result<(), Error> {
            todo!()
        }

        async fn on_connect(&mut self) -> Result<(), ezsockets::Error> {
            if let Some(on_connected) = self.on_connected.take() {
                on_connected.send(()).ok();
            }
            Ok(())
        }
    }

    #[async_trait]
    impl Transport for ClientTransport {
        async fn send(&self, data: &TransportMessage) -> anyhow::Result<()> {
            let _ = match data {
                TransportMessage::Text(data) => {
                    self.handle.text(Utf8Bytes::try_from(data.clone())?)?
                }
                TransportMessage::Binary(data) => self.handle.binary(Bytes::from(data.clone()))?,
            };
            Ok(())
        }

        async fn receive(&self) -> anyhow::Result<TransportMessage> {
            let mut rx = self.rx.lock().await;
            rx.recv().await.ok_or(anyhow::anyhow!("socket closed"))
        }
    }

    pub async fn connect_transport(
        config: ezsockets::ClientConfig,
    ) -> anyhow::Result<ClientTransport> {
        let (tx_from_socket, rx_from_socket) = unbounded_channel();
        let (tx_connected, rx_connected) = oneshot::channel::<()>();

        let (handle, _) = ezsockets::connect(
            move |_handle| ClientHandler {
                tx: tx_from_socket.clone(),
                on_connected: Some(tx_connected),
            },
            config,
        )
        .await;

        // wait until connected
        rx_connected.await?;

        let transport = ClientTransport {
            handle,
            rx: Mutex::new(rx_from_socket),
        };

        Ok(transport)
    }

    /// Connect to a websocket server and return a started RPC session
    pub async fn connect<C, S>(
        config: ezsockets::ClientConfig,
        codec: C,
        handler: Arc<dyn RpcSessionHandler<State = S>>,
        state: S,
    ) -> anyhow::Result<Arc<RpcSession<C, ClientTransport, S>>>
    where
        C: Codec,
        S: SessionState,
    {
        let transport = connect_transport(config).await?;
        Ok(RpcSession::create(transport, codec, handler, state))
    }
}

pub mod server {
    use crate::codec::Codec;
    use crate::session::{RpcSession, RpcSessionHandler, SessionState};
    use crate::transport::{Transport, TransportMessage};
    use async_trait::async_trait;
    use ezsockets::{
        Bytes, CloseFrame, Error, Request, Server, ServerExt, SessionExt, Socket, Utf8Bytes,
    };
    use std::net::SocketAddr;
    use std::sync::Arc;
    use tokio::sync::Mutex;
    use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
    use tracing::debug;

    type SessionID = u16;
    type Session = ezsockets::Session<SessionID, ()>;

    struct ServerHandler {
        tx_accept: UnboundedSender<ServerSessionTransport>,
    }

    struct ServerSession {
        id: SessionID,
        tx: UnboundedSender<TransportMessage>,
        rx: Option<UnboundedReceiver<TransportMessage>>,
        tx_accept: UnboundedSender<ServerSessionTransport>,
        handle: Session,
    }

    #[async_trait]
    impl SessionExt for ServerSession {
        type ID = SessionID;
        type Call = ();

        fn id(&self) -> &Self::ID {
            &self.id
        }

        async fn on_text(&mut self, text: Utf8Bytes) -> Result<(), Error> {
            self.tx
                .send(TransportMessage::Text(text.as_bytes().into()))?;
            Ok(())
        }

        async fn on_binary(&mut self, bytes: Bytes) -> Result<(), Error> {
            self.tx
                .send(TransportMessage::Binary(bytes.to_vec().into()))?;
            Ok(())
        }

        async fn on_call(&mut self, call: Self::Call) -> Result<(), Error> {
            let t = ServerSessionTransport {
                rx: Mutex::new(self.rx.take().unwrap()),
                handle: self.handle.clone(),
            };

            self.tx_accept.send(t).unwrap();

            Ok(())
        }
    }

    #[async_trait]
    impl ServerExt for ServerHandler {
        type Session = ServerSession;
        type Call = ();

        async fn on_connect(
            &mut self,
            socket: Socket,
            request: Request,
            address: SocketAddr,
        ) -> Result<Session, Option<CloseFrame>> {
            // TODO: validate socket
            // TODO: validate request
            // TODO: auth

            let (tx, rx) = unbounded_channel();

            let id = address.port();
            let session = Session::create(
                |handle| ServerSession {
                    tx,
                    id,
                    rx: Some(rx),
                    tx_accept: self.tx_accept.clone(),
                    handle,
                },
                id,
                socket,
            );

            session.call(()).unwrap();

            Ok(session)
        }

        async fn on_disconnect(
            &mut self,
            id: SessionID,
            reason: Result<Option<CloseFrame>, Error>,
        ) -> Result<(), Error> {
            debug!("session {} disconnected: {:?}", id, reason);
            Ok(())
        }

        async fn on_call(&mut self, call: Self::Call) -> Result<(), Error> {
            todo!()
        }
    }

    pub struct ServerSessionTransport {
        handle: Session,
        rx: Mutex<UnboundedReceiver<TransportMessage>>,
    }

    #[async_trait]
    impl Transport for ServerSessionTransport {
        async fn send(&self, data: &TransportMessage) -> anyhow::Result<()> {
            let _ = match data {
                TransportMessage::Text(data) => {
                    self.handle.text(Utf8Bytes::try_from(data.to_vec())?)?
                }
                TransportMessage::Binary(data) => self.handle.binary(Bytes::from(data.clone()))?,
            };
            Ok(())
        }

        async fn receive(&self) -> anyhow::Result<TransportMessage> {
            let mut rx = self.rx.lock().await;
            rx.recv().await.ok_or(anyhow::anyhow!("socket closed"))
        }
    }

    pub async fn listen<C, S, F, Fut>(
        addr: SocketAddr,
        codec: C,
        handler: Arc<dyn RpcSessionHandler<State = S>>,
        state_factory: F,
    ) -> anyhow::Result<()>
    where
        C: Codec,
        S: SessionState,
        F: Fn() -> Fut + Sync + Send + 'static,
        Fut: Future<Output = Result<S, String>> + Send + 'static,
    {
        let (tx_accept, mut rx_accept) = unbounded_channel();

        tokio::spawn(async move {
            let (server, _) = Server::create(|_server| ServerHandler { tx_accept });
            ezsockets::tungstenite::run(server, addr).await.unwrap();
        });

        // run session per accepted client
        tokio::spawn(async move {
            while let Some(transport) = rx_accept.recv().await {
                let _ = RpcSession::create(
                    transport,
                    codec.clone(),
                    handler.clone(),
                    state_factory().await.unwrap(),
                );
            }
        });

        // TODO: return a handle to stop the server

        Ok(())
    }
}

#[cfg(test)]
mod tests {

    use crate::codec::json::JsonCodec;
    use crate::message::{ErrorBody, Request};
    use crate::session::{RpcSessionHandler, SessionContext};
    use crate::transport::websocket::client::connect;
    use crate::transport::websocket::server::listen;
    use async_trait::async_trait;
    use ezsockets::ClientConfig;
    use nanoid::nanoid;
    use serde_json::Value;
    use std::net::SocketAddr;
    use std::sync::Arc;
    use std::time::Duration;
    use url::Url;

    struct TestHandler {}

    #[async_trait]
    impl RpcSessionHandler for TestHandler {
        type State = ();
        async fn on_request(
            &self,
            s: Arc<dyn SessionContext<State = Self::State>>,
            req: Request,
        ) -> Result<Value, ErrorBody> {
            match req.method.as_str() {
                "ping" => Ok(Value::String("pong".to_string())),
                _ => RpcSessionHandler::on_request(self, s, req).await,
            }
        }
    }

    #[tokio::test]
    async fn client_server() {
        let addr: SocketAddr = "127.0.0.1:8080".parse().unwrap();
        let codec = JsonCodec::new();
        let handler = Arc::new(TestHandler {});

        // server
        let server = listen(
            addr,
            codec.clone(),
            handler.clone(),
            || async move { Ok(()) },
        )
        .await
        .unwrap();

        // client
        let client_url = Url::parse(format!("ws://{}", addr).as_str()).unwrap();
        let client_config = ClientConfig::new(client_url);
        let client = connect(client_config, codec, handler.clone(), ())
            .await
            .unwrap();

        let response = client
            .request(
                &Request {
                    id: nanoid!(),
                    method: "ping".to_string(),
                    params: None,
                },
                Duration::from_millis(100).into(),
            )
            .await
            .unwrap();

        assert_eq!(response.result, Value::String("pong".to_string()));
    }
}
