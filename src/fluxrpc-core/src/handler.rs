use async_trait::async_trait;
use crate::message::{ErrorBody, Event, Request, StandardErrorCode};
use serde_json::Value;

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

#[async_trait]
pub trait RpcHandler: Send + Sync + 'static {
    async fn on_event(&self, evt: Event) {}
    async fn on_request(&self, req: Request) -> Result<Value, ErrorBody> {
        Err(HandlerError::Unimplemented { method: req.method }.into())
    }
}
