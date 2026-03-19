//! Type facade for `runtime/api/types/`.
//! Role-based files are split by concern while preserving the public API surface.

mod command_exec;
mod input;
mod policies;
mod skills;
mod thread_ops;
mod thread_views;

pub use command_exec::{
    CommandExecOutputDeltaNotification, CommandExecOutputStream, CommandExecParams,
    CommandExecResizeParams, CommandExecResizeResponse, CommandExecResponse,
    CommandExecTerminalSize, CommandExecTerminateParams, CommandExecTerminateResponse,
    CommandExecWriteParams, CommandExecWriteResponse,
};
pub use input::TextElement;
pub use input::{ByteRange, InputItem, PromptAttachment, ThreadId, TurnId};
pub(crate) use policies::{
    sandbox_policy_to_wire_value, summarize_sandbox_policy, summarize_sandbox_policy_wire_value,
};
pub use policies::{
    ApprovalPolicy, ExternalNetworkAccess, Personality, ReasoningEffort, SandboxPolicy,
    SandboxPreset, ServiceTier, DEFAULT_REASONING_EFFORT,
};
pub use skills::{
    SkillDependencies, SkillErrorInfo, SkillInterface, SkillMetadata, SkillScope,
    SkillToolDependency, SkillsListEntry, SkillsListExtraRootsForCwd, SkillsListParams,
    SkillsListResponse,
};
pub use thread_ops::{
    ThreadHandle, ThreadListParams, ThreadListResponse, ThreadListSortKey, ThreadLoadedListParams,
    ThreadLoadedListResponse, ThreadReadParams, ThreadRollbackParams, ThreadRollbackResponse,
    ThreadStartParams, TurnHandle, TurnStartParams,
};
pub use thread_views::{
    ThreadAgentMessageItemView, ThreadCommandExecutionItemView, ThreadItemPayloadView,
    ThreadItemType, ThreadItemView, ThreadReadResponse, ThreadTurnErrorView, ThreadTurnStatus,
    ThreadTurnView, ThreadView,
};
