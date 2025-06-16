use nanoid::nanoid;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Message {
    Request(Request),
    Response(Response),
    Event(Event),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub id: String,
    pub method: String,
    pub params: Option<Value>,
}

impl Request {
    pub fn new(method: &str, params: Option<Value>) -> Self {
        Self {
            id: nanoid!(),
            method: method.to_string(),
            params,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Response {
    Ok(RequestResult),
    Error(RequestError),
}

impl Response {
    pub fn id(&self) -> &str {
        match self {
            Response::Ok(result) => &result.id,
            Response::Error(error) => &error.id,
        }
    }

    pub fn is_ok(&self) -> bool {
        matches!(self, Response::Ok(_))
    }

    pub fn is_error(&self) -> bool {
        matches!(self, Response::Error(_))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestResult {
    pub id: String,
    pub result: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestError {
    pub id: String,
    pub error: ErrorBody,
}

impl fmt::Display for RequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "RequestError[code={:?}]: {}",
            self.error.code, self.error.message
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorBody {
    pub code: ErrorCode,
    pub message: String,
    pub data: Option<Value>,
}

impl Into<Result<Value, ErrorBody>> for ErrorBody {
    fn into(self) -> Result<Value, ErrorBody> {
        Err(self)
    }
}

impl ErrorBody {
    pub fn internal_error(message: String) -> Self {
        Self {
            code: StandardErrorCode::InternalError.into(),
            message,
            data: None,
        }
    }

    pub fn timeout() -> Self {
        Self {
            code: StandardErrorCode::RequestTimeout.into(),
            message: "Request timeout".to_string(),
            data: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ErrorCode {
    Standard(StandardErrorCode),
    Custom(i32),
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErrorCode::Standard(code) => write!(f, "{}", code),
            ErrorCode::Custom(code) => write!(f, "{}", code),
        }
    }
}

#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StandardErrorCode {
    NotFound = 404,
    Unauthorized = 401,
    RequestTimeout = 408,
    InternalError = 500,
    NotImplemented = 501,
}

impl Into<ErrorCode> for StandardErrorCode {
    fn into(self) -> ErrorCode {
        ErrorCode::Standard(self)
    }
}

impl fmt::Display for StandardErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub event: String,
    pub data: Option<Value>,
}

impl Event {
    pub fn new<T>(event: &str, data: Option<T>) -> Self
    where
        T: Serialize,
    {
        Self {
            event: event.to_string(),
            data: serde_json::to_value(data).ok(),
        }
    }
}
