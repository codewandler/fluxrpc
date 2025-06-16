use crate::Event;
use crate::message::{ErrorBody, Request};
use crate::session::{HandlerError, RpcSessionHandler, SessionHandle, SessionState};
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::Arc;
use tracing::trace;

#[async_trait]
trait TypedRpcMethod: Send + Sync {
    type State: SessionState;
    async fn call(
        &self,
        s: Arc<dyn SessionHandle<State = Self::State>>,
        params: Value,
    ) -> Result<Value, ErrorBody>;
}

#[async_trait]
trait TypedRpcDispatch: Send + Sync {
    type State: SessionState;
    async fn call(
        &self,
        s: Arc<dyn SessionHandle<State = Self::State>>,
        params: Value,
    ) -> anyhow::Result<()>;
}

#[async_trait]
trait Callable: Send + Sync {
    type State: SessionState;
    type Input;
    async fn call(
        &self,
        s: Arc<dyn SessionHandle<State = Self::State>>,
        data: Self::Input,
    ) -> anyhow::Result<()>;
}

pub struct TypedRpcHandler<S>
where
    S: SessionState,
{
    request_handlers: HashMap<String, Arc<dyn TypedRpcMethod<State = S>>>,
    event_handlers: HashMap<String, Arc<dyn TypedRpcDispatch<State = S>>>,
    data_handler: Option<Arc<dyn Callable<State = S, Input = Vec<u8>>>>,
    open_handler: Option<Arc<dyn Callable<State = S, Input = ()>>>,
    close_handler: Option<Arc<dyn Callable<State = S, Input = ()>>>,
    _state: PhantomData<S>,
}

impl<S> TypedRpcHandler<S>
where
    S: SessionState,
{
    pub fn new() -> Self {
        Self {
            request_handlers: HashMap::new(),
            event_handlers: HashMap::new(),
            data_handler: None,
            open_handler: None,
            close_handler: None,
            _state: PhantomData,
        }
    }

    pub fn with_open_handler<F, Fut>(&mut self, f: F)
    where
        F: Fn(Arc<dyn SessionHandle<State = S>>, ()) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), anyhow::Error>> + Send + 'static,
    {
        let handler = Arc::new(TypedCallbackHandler::<F, S, ()> {
            f: Arc::new(f),
            _phantom: Default::default(),
        });

        self.open_handler.replace(handler);
    }

    pub fn register_event_handler<E, F, Fut>(&mut self, name: &str, f: F)
    where
        E: Serialize + DeserializeOwned + Send + Sync + schemars::JsonSchema + 'static,
        F: Fn(Arc<dyn SessionHandle<State = S>>, E) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), anyhow::Error>> + Send + 'static,
    {
        let handler = Arc::new(TypedEventHandler::<E, F, S> {
            event: name.to_string(),
            f: Arc::new(f),
            _phantom: Default::default(),
        });

        self.event_handlers.insert(name.to_string(), handler);
    }

    pub fn register_data_handler<F, Fut>(&mut self, f: F)
    where
        F: Fn(Arc<dyn SessionHandle<State = S>>, Vec<u8>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), anyhow::Error>> + Send + 'static,
    {
        let handler = Arc::new(TypedCallbackHandler::<F, S, Vec<u8>> {
            f: Arc::new(f),
            _phantom: Default::default(),
        });

        self.data_handler.replace(handler);
    }

    pub fn register_request_handler<Req, Res, Err, F, Fut>(&mut self, method: &str, f: F)
    where
        Req: DeserializeOwned + JsonSchema + Sync + Send + 'static,
        Res: Serialize + JsonSchema + Sync + Send + 'static,
        Err: Into<ErrorBody> + Sync + Send + 'static,
        F: Fn(Arc<dyn SessionHandle<State = S>>, Req) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Res, Err>> + Send + 'static,
    {
        let handler = Arc::new(TypedRequestHandler::<Req, Res, Err, F, S> {
            f: Arc::new(f),
            method: method.to_string(),
            _phantom: Default::default(),
        });
        self.request_handlers.insert(method.to_string(), handler);
    }
}

struct TypedRequestHandler<Req, Res, Err, F, S> {
    method: String,
    f: Arc<F>,
    _phantom: PhantomData<(Req, Res, Err, S)>,
}

struct TypedEventHandler<E, F, S> {
    event: String,
    f: Arc<F>,
    _phantom: PhantomData<(E, S)>,
}

struct TypedCallbackHandler<F, S, I> {
    f: Arc<F>,
    _phantom: PhantomData<(S, I)>,
}

#[async_trait]
impl<F, S, I, Fut> Callable for TypedCallbackHandler<F, S, I>
where
    F: Fn(Arc<dyn SessionHandle<State = S>>, I) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = anyhow::Result<()>> + Send + 'static,
    S: SessionState,
    I: Sync + Send + 'static,
{
    type State = S;
    type Input = I;

    async fn call(
        &self,
        s: Arc<dyn SessionHandle<State = Self::State>>,
        data: I,
    ) -> anyhow::Result<()> {
        _ = (self.f)(s, data).await;

        Ok(())
    }
}

#[async_trait]
impl<E, F, S, Fut> TypedRpcDispatch for TypedEventHandler<E, F, S>
where
    E: DeserializeOwned + JsonSchema + Sync + Send + 'static,
    F: Fn(Arc<dyn SessionHandle<State = S>>, E) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = anyhow::Result<()>> + Send + 'static,
    S: SessionState,
{
    type State = S;

    async fn call(
        &self,
        s: Arc<dyn SessionHandle<State = Self::State>>,
        params: Value,
    ) -> anyhow::Result<()> {
        let input: E = serde_json::from_value(params)?;

        _ = (self.f)(s, input).await;

        Ok(())
    }
}

#[async_trait]
impl<Req, Res, Err, F, Fut, S> TypedRpcMethod for TypedRequestHandler<Req, Res, Err, F, S>
where
    Req: DeserializeOwned + JsonSchema + Sync + Send + 'static,
    Res: Serialize + JsonSchema + Sync + Send + 'static,
    Err: Into<ErrorBody> + Sync + Send + 'static,
    F: Fn(Arc<dyn SessionHandle<State = S>>, Req) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<Res, Err>> + Send + 'static,
    S: SessionState,
{
    type State = S;

    async fn call(
        &self,
        s: Arc<dyn SessionHandle<State = Self::State>>,
        params: Value,
    ) -> Result<Value, ErrorBody> {
        let input: Req = serde_json::from_value(params)
            .map_err(|e| ErrorBody::internal_error(format!("Invalid params: {}", e)))?;

        let result = (self.f)(s, input).await;

        match result {
            Ok(output) => serde_json::to_value(output)
                .map_err(|e| ErrorBody::internal_error(format!("Serialization error: {}", e))),
            Err(e) => Err(e.into()),
        }
    }
}

#[async_trait]
impl<S> RpcSessionHandler for TypedRpcHandler<S>
where
    S: SessionState,
{
    type State = S;

    async fn on_open(&self, s: Arc<dyn SessionHandle<State = S>>) -> anyhow::Result<()> {
        if let Some(handler) = &self.open_handler {
            handler.call(s, ()).await?;
        }
        Ok(())
    }

    async fn on_data(
        &self,
        s: Arc<dyn SessionHandle<State = Self::State>>,
        data: Vec<u8>,
    ) -> anyhow::Result<()> {
        if let Some(handler) = &self.data_handler {
            handler.call(s, data).await?;
        }
        Ok(())
    }

    async fn on_event(
        &self,
        s: Arc<dyn SessionHandle<State = S>>,
        evt: Event,
    ) -> anyhow::Result<()> {
        match self.event_handlers.get(&evt.event) {
            Some(handler) => handler.call(s, evt.data.unwrap_or(Value::Null)).await,
            None => {
                trace!("Unhandled event: {:?}", evt);
                Ok(())
            }
        }
    }

    async fn on_request(
        &self,
        s: Arc<dyn SessionHandle<State = S>>,
        req: Request,
    ) -> Result<Value, ErrorBody> {
        match self.request_handlers.get(&req.method) {
            Some(handler) => handler.call(s, req.params.unwrap_or(Value::Null)).await,
            None => Err(HandlerError::Unimplemented { method: req.method }.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::json::JsonCodec;
    use crate::message::Event;
    use crate::transport::channel::new_channel_sessions;
    use schemars::JsonSchema;
    use serde::Deserialize;

    #[derive(Debug, Serialize, Deserialize, JsonSchema)]
    struct MyRequest {
        a: i32,
        b: String,
        c: Option<f64>,
    }

    #[derive(Debug, Serialize, Deserialize, JsonSchema)]
    struct MyResponse {
        status: bool,
    }

    #[derive(Debug, Serialize, Deserialize, JsonSchema)]
    struct MyCustomEvent {
        a: i32,
        b: String,
        c: Option<f64>,
    }

    #[tokio::test]
    async fn test_registry() {
        let mut registry = TypedRpcHandler::new();

        registry.with_open_handler(|s, _| async move {
            println!("OPEN!");
            Ok(())
        });

        registry.register_request_handler::<f32, f32, ErrorBody, _, _>(
            "test",
            |s, x: f32| async move { Ok(x * 2.0) },
        );
        registry.register_request_handler::<MyRequest, MyResponse, ErrorBody, _, _>(
            "some_request",
            |_, _| async move { Ok(MyResponse { status: true }) },
        );

        registry.register_event_handler("my_event", |_, evt: MyCustomEvent| async move {
            println!("Event: {:?}", evt);
            Ok(())
        });

        registry.register_data_handler(|s, data| async move {
            println!("Data: {:?}", data);
            Ok(())
        });

        let (s1, s2) = new_channel_sessions(JsonCodec::new(), Arc::new(registry), (), ());

        s1.notify(&Event::new(
            "my_event",
            MyCustomEvent {
                a: 0,
                b: "".to_string(),
                c: None,
            }
            .into(),
        ))
        .await
        .unwrap();
    }
}
