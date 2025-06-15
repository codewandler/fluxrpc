use crate::message::{ErrorBody, Request};
use crate::schema;
use crate::session::{HandlerError, RpcSessionHandler};
use async_trait::async_trait;
use schemars::{Schema, json_schema, schema_for};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;

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
    fn schema(&self) -> MethodSchema;
    async fn call(&self, params: Value) -> Result<Value, ErrorBody>;
}

pub struct RpcRegistry {
    methods: HashMap<String, Arc<dyn TypedRpcMethod>>,
    events: HashMap<String, EventSchema>,
}

impl RpcRegistry {
    pub fn new() -> Self {
        Self {
            methods: HashMap::new(),
            events: HashMap::new(),
        }
    }

    pub fn register_event<E: schemars::JsonSchema + 'static>(&mut self, name: &str) {
        let schema = schema_for!(E);
        self.events.insert(
            name.to_string(),
            EventSchema {
                name: name.to_string(),
                schema,
            },
        );
    }

    pub fn register_method<Req, Res, Err, F, Fut>(&mut self, method: &str, f: F)
    where
        Req: serde::de::DeserializeOwned + schemars::JsonSchema + Sync + Send + 'static,
        Res: serde::Serialize + schemars::JsonSchema + Sync + Send + 'static,
        Err: Into<ErrorBody> + Sync + Send + 'static,
        F: Fn(Req) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<Res, Err>> + Send + 'static,
    {
        let handler = Arc::new(TypedHandler {
            f: Arc::new(f),
            method: method.to_string(),
            _phantom: Default::default(),
        });
        self.methods.insert(method.to_string(), handler);
    }

    pub fn schema(&self) -> schema::Schema {
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
        let requests: HashMap<_, _> = HashMap::from_iter(self.methods.iter().map(|(k, v)| {
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
    }
}

struct TypedHandler<Req, Res, Err, F> {
    method: String,
    f: Arc<F>,
    _phantom: std::marker::PhantomData<(Req, Res, Err)>,
}

#[async_trait]
impl<Req, Res, Err, F, Fut> TypedRpcMethod for TypedHandler<Req, Res, Err, F>
where
    Req: serde::de::DeserializeOwned + schemars::JsonSchema + Sync + Send + 'static,
    Res: serde::Serialize + schemars::JsonSchema + Sync + Send + 'static,
    Err: Into<ErrorBody> + Sync + Send + 'static,
    F: Fn(Req) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<Res, Err>> + Send + 'static,
{
    fn schema(&self) -> MethodSchema {
        MethodSchema {
            method: self.method.clone(),
            params: schema_for!(Req),
            result: schema_for!(Res),
        }
    }

    async fn call(&self, params: Value) -> Result<Value, ErrorBody> {
        let input: Req = serde_json::from_value(params)
            .map_err(|e| ErrorBody::internal_error(format!("Invalid params: {}", e)))?;

        let result = (self.f)(input).await;

        match result {
            Ok(output) => serde_json::to_value(output)
                .map_err(|e| ErrorBody::internal_error(format!("Serialization error: {}", e))),
            Err(e) => Err(e.into()),
        }
    }
}

#[async_trait]
impl RpcSessionHandler for RpcRegistry {
    async fn on_request(&self, req: Request) -> Result<Value, ErrorBody> {
        let method = self.methods.get(&req.method);
        match method {
            Some(handler) => handler.call(req.params.unwrap_or(Value::Null)).await,
            None => Err(HandlerError::Unimplemented { method: req.method }.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        let mut registry = RpcRegistry::new();

        registry.register_method::<f32, f32, ErrorBody, _, _>("test", |x: f32| async move {
            Ok(x * 2.0)
        });
        registry.register_method::<MyRequest, MyResponse, ErrorBody, _, _>(
            "some_request",
            |_| async move { Ok(MyResponse { status: true }) },
        );

        registry.register_event::<MyCustomEvent>("my_event");

        let req = Request::new("test", Value::from(1.0).into());
        let res = registry.on_request(req).await.unwrap();
        assert_eq!(res, Value::from(2.0));

        println!(
            "{}",
            serde_json::to_string_pretty(&registry.schema()).unwrap()
        );
    }
}
