use crate::codec::Codec;
use crate::message::{
    ErrorBody, Event, Message, Request, RequestError, RequestResult, Response, StandardErrorCode,
};
use crate::transport::{Transport, TransportMessage};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, oneshot};
use tokio::time;
use tracing::{debug, error};

pub trait SessionState: Send + Sync + 'static {}

impl SessionState for () {}

#[derive(Debug)]
pub enum RpcSessionError {
    Transport(anyhow::Error),
    Request(RequestError),
}

impl fmt::Display for RpcSessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RpcSessionError::Transport(e) => write!(f, "transport error: {}", e),
            RpcSessionError::Request(e) => write!(f, "request error: {}", e),
        }
    }
}

impl std::error::Error for RpcSessionError {}

pub enum HandlerError {
    Unimplemented { method: String },
}

impl Into<ErrorBody> for HandlerError {
    fn into(self) -> ErrorBody {
        match self {
            Self::Unimplemented { method } => ErrorBody {
                message: format!("Method [{}] is not implemented", method),
                code: StandardErrorCode::NotImplemented.into(),
                data: None,
            },
        }
    }
}

pub trait SessionHandle: Sync + Send {
    type State: SessionState;

    fn state(&self) -> &Mutex<Self::State>;
}

#[async_trait]
pub trait RpcSessionHandler: Send + Sync + 'static {
    type State: SessionState;

    async fn on_open(&self, s: Arc<dyn SessionHandle<State = Self::State>>) -> anyhow::Result<()> {
        Ok(())
    }
    async fn on_close(&self, s: Arc<dyn SessionHandle<State = Self::State>>) -> anyhow::Result<()> {
        Ok(())
    }
    async fn on_data(
        &self,
        s: Arc<dyn SessionHandle<State = Self::State>>,
        data: Vec<u8>,
    ) -> anyhow::Result<()> {
        Ok(())
    }
    async fn on_event(
        &self,
        s: Arc<dyn SessionHandle<State = Self::State>>,
        evt: Event,
    ) -> anyhow::Result<()> {
        Ok(())
    }
    async fn on_request(
        &self,
        s: Arc<dyn SessionHandle<State = Self::State>>,
        req: Request,
    ) -> Result<Value, ErrorBody> {
        Err(HandlerError::Unimplemented { method: req.method }.into())
    }
}

pub struct RpcSession<C, T, S>
where
    C: Codec,
    T: Transport,
    S: SessionState,
{
    state: Arc<Mutex<S>>,
    transport: Arc<T>,
    codec: C,
    pending_requests: Arc<Mutex<HashMap<String, oneshot::Sender<Response>>>>,
    handler: Arc<dyn RpcSessionHandler<State = S>>,
    _foo: std::marker::PhantomData<S>,
}

impl<C, T, S> RpcSession<C, T, S>
where
    T: Transport,
    C: Codec,
    S: SessionState,
{
    pub fn create(
        transport: T,
        codec: C,
        handler: Arc<dyn RpcSessionHandler<State = S>>,
        state: S,
    ) -> Arc<Self> {
        let s = Arc::new(Self {
            codec,
            transport: Arc::new(transport),
            pending_requests: Arc::new(Mutex::new(HashMap::new())),
            handler,
            state: Arc::new(Mutex::new(state)),
            _foo: std::marker::PhantomData,
        });

        let s1 = s.clone();
        tokio::spawn(async move {
            s1.start().await;
        });

        s
    }

    pub async fn start(self: Arc<Self>) {
        let session = self.clone();
        tokio::spawn(async move {
            session.run().await;
        });
    }

    pub async fn notify(&self, event: &Event) -> anyhow::Result<()> {
        let msg = Message::Event(event.clone());
        let data = self.codec.encode(&msg)?;
        self.transport.send(&TransportMessage::Text(data)).await
    }

    pub async fn request(
        &self,
        request: &Request,
        timeout: Option<Duration>,
    ) -> Result<RequestResult, RpcSessionError> {
        let started_at = time::Instant::now();

        debug!("Sending request {request:?}");
        let msg = Message::Request(request.clone());
        let data = self
            .codec
            .encode(&msg)
            .map_err(|err| RpcSessionError::Transport(err))?;

        let (tx, rx) = oneshot::channel();
        let id = request.id.clone();
        {
            self.pending_requests.lock().await.insert(id.clone(), tx);
        }

        self.transport
            .send(&TransportMessage::Text(data))
            .await
            .map_err(RpcSessionError::Transport)?;

        let result = match timeout {
            Some(dur) => time::timeout(dur, rx).await.map_err(|_| {
                RpcSessionError::Request(RequestError {
                    id: id.clone(),
                    error: ErrorBody::timeout(),
                })
            })?,
            None => rx.await,
        };

        let took = started_at.elapsed().as_micros();
        debug!("Request {request:?} took {took} microseconds");

        match result {
            Ok(Response::Ok(r)) => Ok(r),
            Ok(Response::Error(e)) => Err(RpcSessionError::Request(e)),
            Err(err) => Err(RpcSessionError::Request(RequestError {
                id: id.clone(),
                error: ErrorBody::internal_error(err.to_string()),
            })),
        }
    }

    async fn run(self: Arc<Self>) {
        let pending = self.pending_requests.clone();
        let codec = self.codec.clone();
        let handler = self.handler.clone();
        let transport = self.transport.clone();
        //let state = self.state.clone();
        let handle: Arc<dyn SessionHandle<State = S>> = self.clone();

        self.handler
            .on_open(handle.clone())
            .await
            .expect("TODO: panic message");

        tokio::spawn(async move {
            let transport = transport.clone();
            //let state = state.clone();
            loop {
                let data = match transport.receive().await {
                    Ok(d) => d,
                    Err(err) => {
                        error!("Transport receive error: {err}");
                        break;
                    }
                };

                match data {
                    TransportMessage::Binary(data) => {
                        if let Err(err) = handler.on_data(handle.clone(), data).await {
                            error!("Data handler error: {err}");
                        }
                    }
                    TransportMessage::Text(data) => {
                        let msg: Message = match codec.decode(&data) {
                            Ok(m) => m,
                            Err(err) => {
                                error!("Decode error: {err}");
                                continue;
                            }
                        };

                        match &msg {
                            Message::Response(res) => match pending.lock().await.remove(res.id()) {
                                Some(tx) => tx.send(res.clone()).expect("failed to send response"),
                                None => {
                                    error!("received response for unknown request: {{res.id()}}");
                                }
                            },
                            Message::Event(evt) => {
                                match handler.on_event(handle.clone(), evt.clone()).await {
                                    Err(err) => error!("Event handler error: {err}"),
                                    Ok(()) => {}
                                }
                            }
                            Message::Request(req) => {
                                let request_id = req.id.clone();
                                let res: Response =
                                    match handler.on_request(handle.clone(), req.clone()).await {
                                        Ok(v) => Response::Ok(RequestResult {
                                            id: request_id,
                                            result: v,
                                        }),
                                        Err(err) => Response::Error(RequestError {
                                            id: request_id,
                                            error: err.into(),
                                        }),
                                    };
                                let msg = Message::Response(res);
                                let data = codec.encode(&msg).expect("failed to encode response");
                                transport
                                    .send(&TransportMessage::Text(data))
                                    .await
                                    .expect("failed to send response");
                            }
                        }
                    }
                }
            }
        });
    }
}

impl<C, T, S> SessionHandle for RpcSession<C, T, S>
where
    C: Codec,
    T: Transport,
    S: SessionState,
{
    type State = S;

    fn state(&self) -> &Mutex<Self::State> {
        self.state.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::json::JsonCodec;
    use crate::transport::channel::channel_transport_pair;
    use async_trait::async_trait;
    use serde_json::{Value, json};

    struct MyHandler;

    #[async_trait]
    impl RpcSessionHandler for MyHandler {
        type State = ();
        async fn on_request(
            &self,
            s: Arc<dyn SessionHandle<State = Self::State>>,
            req: Request,
        ) -> Result<Value, ErrorBody> {
            assert_eq!(req.method, "ping");
            Ok(json!("pong"))
        }
    }

    #[tokio::test]
    async fn test_request_response() {
        let handler = Arc::new(MyHandler);
        let (a, b) = channel_transport_pair(10);
        let session_a = RpcSession::create(a, JsonCodec::new(), handler.clone(), ());
        let session_b = RpcSession::create(b, JsonCodec::new(), handler.clone(), ());

        let req = Request::new("ping", None);

        let res = session_a
            .request(&req, Some(Duration::from_millis(100)))
            .await
            .expect("request failed");

        assert_eq!(res.id, req.id);
        assert_eq!(res.result, json!("pong"));
    }
}
