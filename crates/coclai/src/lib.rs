//! Public facade for the coclai workspace.
//! Default path: use this crate first. Use `coclai::runtime` for low-level control.

mod appserver;
mod ergonomic;

pub use appserver::{methods as rpc_methods, AppServer};
pub use coclai_runtime::{
    ApprovalPolicy, Client, ClientConfig, ClientError, CompatibilityGuard, HookAction,
    HookAttachment, HookContext, HookIssue, HookIssueClass, HookPatch, HookPhase, HookReport,
    PluginContractVersion, PostHook, PreHook, PromptAttachment, PromptRunError, PromptRunParams,
    PromptRunResult, ReasoningEffort, RpcError, RpcErrorObject, RpcValidationMode, RunProfile,
    RuntimeError, RuntimeHookConfig, SandboxPolicy, SandboxPreset, SemVerTriplet, ServerRequest,
    ServerRequestRx, Session, SessionConfig, ThreadAgentMessageItemView,
    ThreadCommandExecutionItemView, ThreadItemPayloadView, ThreadItemType, ThreadItemView,
    ThreadListParams, ThreadListResponse, ThreadListSortKey, ThreadLoadedListParams,
    ThreadLoadedListResponse, ThreadReadParams, ThreadReadResponse, ThreadRollbackParams,
    ThreadRollbackResponse, ThreadTurnErrorView, ThreadTurnStatus, ThreadTurnView, ThreadView,
    DEFAULT_REASONING_EFFORT,
};
pub use ergonomic::{quick_run, quick_run_with_profile, QuickRunError, Workflow, WorkflowConfig};

pub use coclai_runtime as runtime;
