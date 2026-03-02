#[cfg(test)]
use std::str::FromStr;

use crate::errors::RpcError;
use crate::runtime::Runtime;
use crate::turn_output::parse_thread_id;

mod flow;
mod models;
mod ops;
mod prompt_run;
mod thread_api;
mod turn_error;
mod wire;

#[cfg(test)]
use wire::{build_prompt_inputs, validate_prompt_attachments};
#[cfg(test)]
use wire::{input_item_to_wire, turn_start_params_to_wire};
use wire::{thread_start_params_to_wire, validate_thread_start_security};

mod types;

pub use models::{
    PromptRunError, PromptRunParams, PromptRunResult, PromptTurnFailure, PromptTurnTerminalState,
};
pub use types::*;

impl Runtime {
    pub(crate) async fn thread_start_raw(
        &self,
        p: ThreadStartParams,
    ) -> Result<ThreadHandle, RpcError> {
        validate_thread_start_security(&p)?;
        let response = self
            .call_raw("thread/start", thread_start_params_to_wire(&p))
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
