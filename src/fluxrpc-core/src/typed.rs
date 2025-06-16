use crate::Event;
use crate::message::{ErrorBody, Request};
use crate::session::{HandlerError, RpcSessionHandler, SessionState};
use async_trait::async_trait;
use schemars::Schema;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::sync::Arc;
use tracing::trace;

#[derive(Debug, Serialize)]
struct MethodSchema {
    pub method: String,
    pub params: Schema,
    pub result: Schema,
}

#[derive(Debug, Serialize)]
struct EventSchema {
    pub name: String,
    pub schema: Schema,
}

#[async_trait]
trait TypedRpcMethod: Send + Sync {
    type State: SessionState;

    //fn schema(&self) -> MethodSchema;
    async fn call(&self, s: Self::State, params: Value) -> Result<Value, ErrorBody>;
}

#[async_trait]
trait TypedRpcDispatch: Send + Sync {
    type State: SessionState;
    async fn call(&self, s: Self::State, params: Value) -> anyhow::Result<()>;
}

#[async_trait]
trait TypedDataDispatch: Send + Sync {
    type State: SessionState;
    async fn call(&self, s: Self::State, data: Vec<u8>) -> anyhow::Result<()>;
}

pub struct TypedRpcHandler<S>
where
    S: SessionState,
{
    request_handlers: HashMap<String, Arc<dyn TypedRpcMethod<State = S>>>,
    event_handlers: HashMap<String, Arc<dyn TypedRpcDispatch<State = S>>>,
    data_handler: Option<Arc<dyn TypedDataDispatch<State = S>>>,
    //events: HashMap<String, EventSchema>,
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
            //events: HashMap::new(),
            _state: PhantomData,
        }
    }

    pub fn register_event_handler<E, F, Fut>(&mut self, name: &str, f: F)
    where
        E: Serialize + DeserializeOwned + Send + Sync + schemars::JsonSchema + 'static,
        F: Fn(S, E) -> Fut + Send + Sync + 'static,
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
        F: Fn(S, Vec<u8>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), anyhow::Error>> + Send + 'static,
    {
        let handler = Arc::new(TypedDataHandler::<F, S> {
            f: Arc::new(f),
            _phantom: Default::default(),
        });

        self.data_handler.replace(handler);
    }

    pub fn register_request_handler<Req, Res, Err, F, Fut>(&mut self, method: &str, f: F)
    where
        Req: serde::de::DeserializeOwned + schemars::JsonSchema + Sync + Send + 'static,
        Res: serde::Serialize + schemars::JsonSchema + Sync + Send + 'static,
        Err: Into<ErrorBody> + Sync + Send + 'static,
        F: Fn(S, Req) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Res, Err>> + Send + 'static,
    {
        let handler = Arc::new(TypedRequestHandler::<Req, Res, Err, F, S> {
            f: Arc::new(f),
            method: method.to_string(),
            _phantom: Default::default(),
        });
        self.request_handlers.insert(method.to_string(), handler);
    }

    /*pub fn schema(&self) -> schema::Schema {
        let events: HashMap<_, _> = HashMap::from_iter(self.events.iter().map(|(k, v)| {
            (
                k.to_string(),
                json_schema!({
                    "properties": {
                        "event": {
                            "type": "string",
                            "const": v.name.clone(),
                        },
                        "data": v.schema.clone(),
                    },
                    "required": ["event"],
                }),
            )
        }));
        let requests: HashMap<_, _> =
            HashMap::from_iter(self.request_handlers.iter().map(|(k, v)| {
                (
                    k.to_string(),
                    json_schema!({
                        "properties": {
                            "id": {
                                "type": "string"
                            },
                            "method": {
                                "type": "string",
                                "const": k.to_string(),
                            },
                            "params": v.schema(),
                        },
                        "required": ["id", "method"],

                    }),
                )
            }));
        serde_json::from_value(json!({
            "components": {
                "events": events,
                "requests": requests
            }
        }))
        .unwrap()
    }*/
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

struct TypedDataHandler<F, S> {
    f: Arc<F>,
    _phantom: PhantomData<S>,
}

#[async_trait]
impl<F, S, Fut> TypedDataDispatch for TypedDataHandler<F, S>
where
    F: Fn(S, Vec<u8>) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = anyhow::Result<()>> + Send + 'static,
    S: SessionState,
{
    type State = S;

    async fn call(&self, s: Self::State, data: Vec<u8>) -> anyhow::Result<()> {
        _ = (self.f)(s, data).await;

        Ok(())
    }
}

#[async_trait]
impl<E, F, S, Fut> TypedRpcDispatch for TypedEventHandler<E, F, S>
where
    E: serde::de::DeserializeOwned + schemars::JsonSchema + Sync + Send + 'static,
    F: Fn(S, E) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = anyhow::Result<()>> + Send + 'static,
    S: SessionState,
{
    type State = S;

    async fn call(&self, s: Self::State, params: Value) -> anyhow::Result<()> {
        let input: E = serde_json::from_value(params)?;

        _ = (self.f)(s, input).await;

        Ok(())
    }
}

#[async_trait]
impl<Req, Res, Err, F, Fut, S> TypedRpcMethod for TypedRequestHandler<Req, Res, Err, F, S>
where
    Req: serde::de::DeserializeOwned + schemars::JsonSchema + Sync + Send + 'static,
    Res: serde::Serialize + schemars::JsonSchema + Sync + Send + 'static,
    Err: Into<ErrorBody> + Sync + Send + 'static,
    F: Fn(S, Req) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<Res, Err>> + Send + 'static,
    S: SessionState,
{
    type State = S;

    /*fn schema(&self) -> MethodSchema {
        MethodSchema {
            method: self.method.clone(),
            params: schema_for!(Req),
            result: schema_for!(Res),
        }
    }*/

    async fn call(&self, s: S, params: Value) -> Result<Value, ErrorBody> {
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

    async fn on_data(&self, s: Self::State, data: Vec<u8>) -> anyhow::Result<()> {
        if let Some(handler) = &self.data_handler {
            handler.call(s, data).await?;
        }
        Ok(())
    }

    // TODO: event handler
    // TODO: data handler

    async fn on_event(&self, s: Self::State, evt: Event) -> anyhow::Result<()> {
        match self.event_handlers.get(&evt.event) {
            Some(handler) => handler.call(s, evt.data.unwrap_or(Value::Null)).await,
            None => {
                trace!("Unhandled event: {:?}", evt);
                Ok(())
            }
        }
    }

    async fn on_request(&self, s: S, req: Request) -> Result<Value, ErrorBody> {
        match self.request_handlers.get(&req.method) {
            Some(handler) => handler.call(s, req.params.unwrap_or(Value::Null)).await,
            None => Err(HandlerError::Unimplemented { method: req.method }.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::Event;
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

        registry.register_request_handler::<f32, f32, ErrorBody, _, _>(
            "test",
            |_: (), x: f32| async move { Ok(x * 2.0) },
        );
        registry.register_request_handler::<MyRequest, MyResponse, ErrorBody, _, _>(
            "some_request",
            |_, _| async move { Ok(MyResponse { status: true }) },
        );

        registry.register_event_handler("my_event", |_, evt: MyCustomEvent| async move {
            println!("Event: {:?}", evt);
            Ok(())
        });

        registry.register_data_handler(|_: (), data| async move {
            println!("Data: {:?}", data);
            Ok(())
        });

        // 1. handle request
        let req = Request::new("test", Value::from(1.0).into());
        let res = registry.on_request((), req).await.unwrap();
        assert_eq!(res, Value::from(2.0));

        // 2. event
        registry
            .on_event(
                (),
                Event::new(
                    "my_event",
                    MyCustomEvent {
                        a: 0,
                        b: "".to_string(),
                        c: None,
                    }
                    .into(),
                ),
            )
            .await
            .expect("TODO: panic message");

        // 3. data
        registry.on_data((), Vec::new()).await.expect("failed")

        /*println!(
            "{}",
            serde_json::to_string_pretty(&registry.schema()).unwrap()
        );*/
    }
}
