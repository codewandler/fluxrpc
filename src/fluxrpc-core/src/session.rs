use crate::codec::Codec;
use crate::handler::RpcHandler;
use crate::message::{ErrorBody, Event, Message, Request, RequestError, RequestResult, Response};
use crate::transport::Transport;
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, oneshot};
use tokio::time;
use tracing::{debug, error};

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

pub struct RpcSession<T, C>
where
    T: Transport,
    C: Codec,
{
    transport: Arc<T>,
    codec: C,
    pending_requests: Arc<Mutex<HashMap<String, oneshot::Sender<Response>>>>,
    handler: Arc<dyn RpcHandler>,
}

impl<T, C> RpcSession<T, C>
where
    T: Transport,
    C: Codec + Sync + Send + 'static,
{
    pub fn new(transport: T, codec: C, handler: Arc<dyn RpcHandler>) -> Self {
        Self {
            transport: Arc::new(transport),
            codec,
            pending_requests: Arc::new(Mutex::new(HashMap::new())),
            handler,
        }
    }

    pub async fn notify(&mut self, event: &Event) -> anyhow::Result<()> {
        let msg = Message::Event(event.clone());
        let data = self.codec.encode(&msg)?;
        self.transport.send(&data).await
    }

    pub async fn request(
        &self,
        request: &Request,
        timeout: Option<Duration>,
    ) -> Result<RequestResult, RpcSessionError> {
        debug!("Sending request {{request}}");
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
            .send(&data)
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

        match result {
            Ok(Response::Ok(r)) => Ok(r),
            Ok(Response::Error(e)) => Err(RpcSessionError::Request(e)),
            Err(_) => Err(RpcSessionError::Request(RequestError {
                id: id.clone(),
                error: ErrorBody::internal_error("Response channel closed".to_string()),
            })),
        }
    }

    pub async fn start(&self) {
        let pending = self.pending_requests.clone();
        let codec = self.codec.clone();
        let handler = self.handler.clone();
        let transport = self.transport.clone();
        tokio::spawn(async move {
            let transport = transport.clone();
            loop {
                let data = match transport.receive().await {
                    Ok(d) => d,
                    Err(err) => {
                        error!("Transport receive error: {err}");
                        break;
                    }
                };

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
                    Message::Event(evt) => handler.on_event(evt.clone()),
                    Message::Request(req) => {
                        let request_id = req.id.clone();
                        let res: Response = match handler.on_request(req.clone()) {
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
                            .send(&data)
                            .await
                            .expect("failed to send response");
                    }
                }

                // TODO: Handle requests and events here if needed
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::json::JsonCodec;
    use crate::transport::channel::channel_transport_pair;
    use serde_json::{Value, json};

    struct MyHandler;
    impl RpcHandler for MyHandler {
        fn on_request(&self, req: Request) -> Result<Value, ErrorBody> {
            assert_eq!(req.method, "ping");
            Ok(json!("pong"))
        }
    }

    #[tokio::test]
    async fn test_request_response() {
        let handler = Arc::new(MyHandler);
        let (a, b) = channel_transport_pair(10);
        let session_a = RpcSession::new(a, JsonCodec, handler.clone());
        let session_b = RpcSession::new(b, JsonCodec, handler.clone());

        session_a.start().await;
        session_b.start().await;

        let req = Request::new("ping", None);

        let res = session_a
            .request(&req, Some(Duration::from_millis(100)))
            .await
            .expect("request failed");

        assert_eq!(res.id, req.id);
        assert_eq!(res.result, json!("pong"));
    }
}
