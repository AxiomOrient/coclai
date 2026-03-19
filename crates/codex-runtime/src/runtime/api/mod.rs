use crate::runtime::core::Runtime;
use crate::runtime::errors::RpcError;
use crate::runtime::rpc_contract::methods;
use crate::runtime::turn_output::parse_thread_id;

mod attachment_validation;
mod command_exec_api;
mod flow;
mod models;
mod prompt_run;
mod thread_api;
pub(crate) mod tool_use_hooks;
mod turn_error;
mod wire;

use std::path::PathBuf;

#[cfg(test)]
use attachment_validation::validate_prompt_attachments;
#[cfg(test)]
use wire::build_prompt_inputs;
#[cfg(test)]
use wire::{input_item_to_wire, turn_start_params_to_wire};
use wire::{thread_start_params_to_wire, validate_thread_start_security};

fn resolve_attachment_path(cwd: &str, path: &str) -> PathBuf {
    let path = PathBuf::from(path);
    if path.is_absolute() {
        path
    } else {
        PathBuf::from(cwd).join(path)
    }
}

mod types;

pub use models::{
    PromptRunError, PromptRunParams, PromptRunResult, PromptRunStream, PromptRunStreamEvent,
    PromptTurnFailure, PromptTurnTerminalState,
};
pub(crate) use types::{
    sandbox_policy_to_wire_value, summarize_sandbox_policy, summarize_sandbox_policy_wire_value,
};
pub use types::{
    ApprovalPolicy, ByteRange, CommandExecOutputDeltaNotification, CommandExecOutputStream,
    CommandExecParams, CommandExecResizeParams, CommandExecResizeResponse, CommandExecResponse,
    CommandExecTerminalSize, CommandExecTerminateParams, CommandExecTerminateResponse,
    CommandExecWriteParams, CommandExecWriteResponse, ExternalNetworkAccess, InputItem,
    Personality, PromptAttachment, ReasoningEffort, SandboxPolicy, SandboxPreset, ServiceTier,
    SkillDependencies, SkillErrorInfo, SkillInterface, SkillMetadata, SkillScope,
    SkillToolDependency, SkillsListEntry, SkillsListExtraRootsForCwd, SkillsListParams,
    SkillsListResponse, TextElement, ThreadAgentMessageItemView, ThreadCommandExecutionItemView,
    ThreadHandle, ThreadId, ThreadItemPayloadView, ThreadItemType, ThreadItemView,
    ThreadListParams, ThreadListResponse, ThreadListSortKey, ThreadLoadedListParams,
    ThreadLoadedListResponse, ThreadReadParams, ThreadReadResponse, ThreadRollbackParams,
    ThreadRollbackResponse, ThreadStartParams, ThreadTurnErrorView, ThreadTurnStatus,
    ThreadTurnView, ThreadView, TurnHandle, TurnId, TurnStartParams, DEFAULT_REASONING_EFFORT,
};

impl Runtime {
    pub(crate) async fn thread_start_raw(
        &self,
        mut p: ThreadStartParams,
    ) -> Result<ThreadHandle, RpcError> {
        p = escalate_approval_if_tool_hooks(self, p);
        validate_thread_start_security(&p)?;
        let response = self
            .call_validated(methods::THREAD_START, thread_start_params_to_wire(&p))
            .await?;
        let thread_id = parse_thread_id(&response).ok_or_else(|| {
            RpcError::InvalidRequest(format!(
                "thread/start missing thread id in result: {response}"
            ))
        })?;
        Ok(ThreadHandle {
            thread_id,
            runtime: self.clone(),
        })
    }
}

/// If the runtime has pre-tool-use hooks, escalate approval policy from Never → Untrusted
/// so that codex sends approval requests that the hook loop can intercept.
/// Pure transform; no I/O. Allocation: none. Complexity: O(1).
fn escalate_approval_if_tool_hooks(
    runtime: &Runtime,
    mut p: ThreadStartParams,
) -> ThreadStartParams {
    if runtime.has_pre_tool_use_hooks()
        && matches!(p.approval_policy, None | Some(ApprovalPolicy::Never))
    {
        p.approval_policy = Some(ApprovalPolicy::Untrusted);
    }
    p
}

#[cfg(test)]
mod tests;
