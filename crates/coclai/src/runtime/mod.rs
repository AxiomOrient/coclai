pub mod api;
pub mod approvals;
pub mod client;
pub mod core;
pub mod errors;
pub mod events;
pub mod hooks;
pub(crate) mod id;
pub mod metrics;
pub mod rpc;
pub mod rpc_contract;
pub(crate) mod runtime_validation;
pub mod sink;
pub mod state;
pub mod transport;
pub(crate) mod turn_lifecycle;
pub mod turn_output;

pub use api::{
    ApprovalPolicy, PromptAttachment, PromptRunError, PromptRunParams, PromptRunResult,
    ReasoningEffort, SandboxPolicy, SandboxPreset, ThreadAgentMessageItemView,
    ThreadCommandExecutionItemView, ThreadItemPayloadView, ThreadItemType, ThreadItemView,
    ThreadListParams, ThreadListResponse, ThreadListSortKey, ThreadLoadedListParams,
    ThreadLoadedListResponse, ThreadReadParams, ThreadReadResponse, ThreadRollbackParams,
    ThreadRollbackResponse, ThreadTurnErrorView, ThreadTurnStatus, ThreadTurnView, ThreadView,
    DEFAULT_REASONING_EFFORT,
};
pub use approvals::{ServerRequest, ServerRequestConfig, TimeoutAction};
pub use client::{
    Client, ClientConfig, ClientError, CompatibilityGuard, RunProfile, SemVerTriplet, Session,
    SessionConfig,
};
pub use core::{RestartPolicy, Runtime, RuntimeConfig, SupervisorConfig};
pub use errors::{RpcError, RpcErrorObject, RuntimeError, SinkError};
pub use hooks::RuntimeHookConfig;
pub use metrics::RuntimeMetricsSnapshot;
pub use rpc_contract::RpcValidationMode;
pub use transport::{StdioProcessSpec, StdioTransportConfig};

pub type ServerRequestRx = tokio::sync::mpsc::Receiver<ServerRequest>;

/// Current time as Unix milliseconds.
/// Allocation: none. Complexity: O(1).
pub(crate) fn now_millis() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_millis() as i64,
        Err(_) => 0,
    }
}
