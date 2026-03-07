use crate::runtime::core::Runtime;
use crate::runtime::errors::RpcError;
use crate::runtime::rpc_contract::methods;
use crate::runtime::turn_output::parse_thread_id;

mod attachment_validation;
mod flow;
mod models;
mod prompt_run;
mod thread_api;
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
    PromptRunError, PromptRunParams, PromptRunResult, PromptTurnFailure, PromptTurnTerminalState,
};
pub(crate) use types::{
    sandbox_policy_to_wire_value, summarize_sandbox_policy, summarize_sandbox_policy_wire_value,
};
pub use types::{
    ApprovalPolicy, ByteRange, ExternalNetworkAccess, InputItem, PromptAttachment, ReasoningEffort,
    SandboxPolicy, SandboxPreset, TextElement, ThreadAgentMessageItemView,
    ThreadCommandExecutionItemView, ThreadHandle, ThreadId, ThreadItemPayloadView, ThreadItemType,
    ThreadItemView, ThreadListParams, ThreadListResponse, ThreadListSortKey,
    ThreadLoadedListParams, ThreadLoadedListResponse, ThreadReadParams, ThreadReadResponse,
    ThreadRollbackParams, ThreadRollbackResponse, ThreadStartParams, ThreadTurnErrorView,
    ThreadTurnStatus, ThreadTurnView, ThreadView, TurnHandle, TurnId, TurnStartParams,
    DEFAULT_REASONING_EFFORT,
};

impl Runtime {
    pub(crate) async fn thread_start_raw(
        &self,
        p: ThreadStartParams,
    ) -> Result<ThreadHandle, RpcError> {
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

#[cfg(test)]
mod tests;
