pub mod api;
pub mod approvals;
pub mod client;
pub mod errors;
pub mod events;
pub mod hooks;
pub mod metrics;
pub mod rpc;
pub mod rpc_contract;
pub mod runtime;
pub(crate) mod runtime_schema;
pub mod schema;
pub mod sink;
pub mod state;
pub mod transport;
pub mod turn_output;

pub use api::{
    ApprovalPolicy, ByteRange, ExternalNetworkAccess, InputItem, PromptAttachment, PromptRunError,
    PromptRunParams, PromptRunResult, PromptTurnFailure, PromptTurnTerminalState, ReasoningEffort,
    SandboxPolicy, SandboxPreset, TextElement, ThreadAgentMessageItemView,
    ThreadCommandExecutionItemView, ThreadHandle, ThreadItemPayloadView, ThreadItemType,
    ThreadItemView, ThreadListParams, ThreadListResponse, ThreadListSortKey,
    ThreadLoadedListParams, ThreadLoadedListResponse, ThreadReadParams, ThreadReadResponse,
    ThreadRollbackParams, ThreadRollbackResponse, ThreadStartParams, ThreadTurnErrorView,
    ThreadTurnStatus, ThreadTurnView, ThreadView, TurnHandle, TurnStartParams,
    DEFAULT_REASONING_EFFORT,
};
pub use approvals::{ServerRequest, ServerRequestConfig, TimeoutAction};
pub use client::{
    Client, ClientConfig, ClientError, CompatibilityGuard, RunProfile, SemVerTriplet, Session,
    SessionConfig, DEFAULT_SCHEMA_RELATIVE_DIR, SCHEMA_DIR_ENV,
};
pub use coclai_plugin_core::{
    HookAction, HookAttachment, HookContext, HookIssue, HookIssueClass, HookPatch, HookPhase,
    HookReport, PluginContractVersion, PostHook, PreHook,
};
pub use errors::{RpcError, RpcErrorObject, RuntimeError, SinkError};
pub use events::{Direction, Envelope, JsonRpcId, MsgKind};
pub use hooks::RuntimeHookConfig;
pub use metrics::RuntimeMetricsSnapshot;
pub use rpc::{classify_message, extract_ids, ExtractedIds};
pub use rpc_contract::RpcValidationMode;
pub use runtime::{RestartPolicy, Runtime, RuntimeConfig, SchemaGuardConfig, SupervisorConfig};
pub use sink::{EventSink, JsonlFileSink};
pub use state::StateProjectionLimits;
pub use state::{ConnectionState, ItemState, RuntimeState, ThreadState, TurnState, TurnStatus};
pub use transport::{StdioProcessSpec, StdioTransportConfig};

pub type ServerRequestRx = tokio::sync::mpsc::Receiver<ServerRequest>;
