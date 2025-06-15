pub mod client {
    use crate::codec::Codec;
    use crate::session::{RpcSession, RpcSessionHandler};
    use crate::transport::Transport;
    use async_trait::async_trait;
    use ezsockets::{Bytes, Client, Error, Utf8Bytes};
    use std::sync::Arc;
    use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
    use tokio::sync::{Mutex, oneshot};

    pub struct ClientHandler {
        tx: UnboundedSender<Vec<u8>>,
        on_connected: Option<oneshot::Sender<()>>,
    }

    pub struct ClientTransport {
        rx: Mutex<UnboundedReceiver<Vec<u8>>>,
        handle: Client<ClientHandler>,
    }

    #[async_trait]
    impl ezsockets::ClientExt for ClientHandler {
        type Call = ();

        async fn on_text(&mut self, text: Utf8Bytes) -> Result<(), Error> {
            self.tx.send(text.as_bytes().into())?;
            Ok(())
        }

        async fn on_binary(&mut self, bytes: Bytes) -> Result<(), Error> {
            todo!()
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
        async fn send(&self, data: &[u8]) -> anyhow::Result<()> {
            let _ = self.handle.text(Utf8Bytes::try_from(data.to_vec())?)?;
            Ok(())
        }

        async fn receive(&self) -> anyhow::Result<Vec<u8>> {
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
    pub async fn connect<C>(
        config: ezsockets::ClientConfig,
        codec: C,
        handler: Arc<dyn RpcSessionHandler>,
    ) -> anyhow::Result<Arc<RpcSession<C, ClientTransport>>>
    where
        C: Codec,
    {
        let transport = connect_transport(config).await?;
        Ok(RpcSession::open(transport, codec, handler))
    }
}

pub mod server {
    use crate::codec::Codec;
    use crate::session::{RpcSession, RpcSessionHandler};
    use crate::transport::Transport;
    use async_trait::async_trait;
    use ezsockets::{
        Bytes, CloseFrame, Error, Request, Server, ServerExt, SessionExt, Socket, Utf8Bytes,
    };
    use std::net::SocketAddr;
    use std::sync::Arc;
    use tokio::sync::Mutex;
    use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

    type SessionID = u16;
    type Session = ezsockets::Session<SessionID, ()>;

    struct ServerHandler {
        tx_accept: UnboundedSender<ServerSessionTransport>,
    }

    struct ServerSession {
        id: SessionID,
        tx: UnboundedSender<Vec<u8>>,
        rx: Option<UnboundedReceiver<Vec<u8>>>,
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
            self.tx.send(text.as_bytes().into())?;
            Ok(())
        }

        async fn on_binary(&mut self, bytes: Bytes) -> Result<(), Error> {
            todo!()
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
            todo!()
        }

        async fn on_call(&mut self, call: Self::Call) -> Result<(), Error> {
            todo!()
        }
    }

    pub struct ServerSessionTransport {
        handle: Session,
        rx: Mutex<UnboundedReceiver<Vec<u8>>>,
    }

    #[async_trait]
    impl Transport for ServerSessionTransport {
        async fn send(&self, data: &[u8]) -> anyhow::Result<()> {
            let _ = self.handle.text(Utf8Bytes::try_from(data.to_vec())?)?;
            Ok(())
        }

        async fn receive(&self) -> anyhow::Result<Vec<u8>> {
            let mut rx = self.rx.lock().await;
            rx.recv().await.ok_or(anyhow::anyhow!("socket closed"))
        }
    }

    pub async fn listen<C>(
        addr: SocketAddr,
        codec: C,
        handler: Arc<dyn RpcSessionHandler>,
    ) -> anyhow::Result<()>
    where
        C: Codec,
    {
        let (tx_accept, mut rx_accept) = unbounded_channel();

        tokio::spawn(async move {
            let (server, _) = Server::create(|_server| ServerHandler { tx_accept });
            ezsockets::tungstenite::run(server, addr).await.unwrap();
        });

        // run session per accepted client
        tokio::spawn(async move {
            while let Some(transport) = rx_accept.recv().await {
                RpcSession::open(transport, codec.clone(), handler.clone());
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
    use crate::session::RpcSessionHandler;
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
        async fn on_request(&self, req: Request) -> Result<Value, ErrorBody> {
            match req.method.as_str() {
                "ping" => Ok(Value::String("pong".to_string())),
                _ => RpcSessionHandler::on_request(self, req).await,
            }
        }
    }

    #[tokio::test]
    async fn client_server() {
        let addr: SocketAddr = "127.0.0.1:8080".parse().unwrap();
        let codec = JsonCodec::new();
        let handler = Arc::new(TestHandler {});

        // server
        let server = listen(addr, codec.clone(), handler.clone()).await.unwrap();

        // client
        let client_url = Url::parse(format!("ws://{}", addr).as_str()).unwrap();
        let client_config = ClientConfig::new(client_url);
        let client = connect(client_config, codec, handler.clone())
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
