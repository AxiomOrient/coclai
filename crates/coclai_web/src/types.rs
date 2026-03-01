use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CreateSessionRequest {
    pub artifact_id: String,
    pub model: Option<String>,
    pub thread_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CreateSessionResponse {
    pub session_id: String,
    pub thread_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CloseSessionResponse {
    pub thread_id: String,
    pub archived: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CreateTurnRequest {
    pub task: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CreateTurnResponse {
    pub turn_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalResponsePayload {
    #[serde(default)]
    pub decision: Option<Value>,
    #[serde(default)]
    pub result: Option<Value>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WebAdapterConfig {
    pub session_event_channel_capacity: usize,
    pub session_approval_channel_capacity: usize,
}

impl Default for WebAdapterConfig {
    fn default() -> Self {
        Self {
            session_event_channel_capacity: 512,
            session_approval_channel_capacity: 128,
        }
    }
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum WebError {
    #[error("invalid session")]
    InvalidSession,
    #[error("runtime already bound to a web adapter")]
    AlreadyBound,
    #[error("invalid approval")]
    InvalidApproval,
    #[error("invalid config: {0}")]
    InvalidConfig(String),
    #[error("invalid turn payload")]
    InvalidTurnPayload,
    #[error("invalid approval payload")]
    InvalidApprovalPayload,
    #[error(
        "incompatible plugin contract: expected=v{expected_major}.{expected_minor} actual=v{actual_major}.{actual_minor}"
    )]
    IncompatibleContract {
        expected_major: u16,
        expected_minor: u16,
        actual_major: u16,
        actual_minor: u16,
    },
    #[error("forbidden")]
    Forbidden,
    #[error("internal error: {0}")]
    Internal(String),
}
