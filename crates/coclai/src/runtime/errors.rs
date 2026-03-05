use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RpcErrorObject {
    pub code: i64,
    pub message: String,
    pub data: Option<Value>,
}

#[derive(Clone, Debug, Error, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RuntimeError {
    #[error("runtime is not initialized")]
    NotInitialized,
    #[error("runtime is already initialized")]
    AlreadyInitialized,
    #[error("invalid config: {0}")]
    InvalidConfig(String),
    #[error("transport is closed")]
    TransportClosed,
    #[error("child process exited")]
    ProcessExited,
    #[error("request timed out")]
    Timeout,
    #[error("server request receiver already taken")]
    ServerRequestReceiverTaken,
    #[error("internal error: {0}")]
    Internal(String),
}

#[derive(Clone, Debug, Error, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RpcError {
    #[error("runtime overloaded")]
    Overloaded,
    #[error("rpc call timed out")]
    Timeout,
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    #[error("method not found: {0}")]
    MethodNotFound(String),
    #[error("server error: {0:?}")]
    ServerError(RpcErrorObject),
    #[error("transport is closed")]
    TransportClosed,
}

#[derive(Clone, Debug, Error, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum SinkError {
    #[error("io error: {0}")]
    Io(String),
    #[error("serialize error: {0}")]
    Serialize(String),
    #[error("internal error: {0}")]
    Internal(String),
}
