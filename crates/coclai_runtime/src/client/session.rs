use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::api::{PromptRunError, PromptRunParams, PromptRunResult};
use crate::errors::RpcError;
use crate::runtime::Runtime;

use super::profile::{merge_hook_configs, profile_to_prompt_params, session_prompt_params};
use super::{RunProfile, SessionConfig};

#[derive(Clone)]
pub struct Session {
    runtime: Runtime,
    pub thread_id: String,
    pub config: SessionConfig,
    closed: Arc<AtomicBool>,
    close_result: Arc<Mutex<Option<Result<(), RpcError>>>>,
}

impl Session {
    pub(super) fn new(runtime: Runtime, thread_id: String, config: SessionConfig) -> Self {
        Self {
            runtime,
            thread_id,
            config,
            closed: Arc::new(AtomicBool::new(false)),
            close_result: Arc::new(Mutex::new(None)),
        }
    }

    /// Returns true when this local session handle is closed.
    /// Allocation: none. Complexity: O(1).
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    /// Continue this session with one prompt.
    /// Side effects: sends thread/resume + turn/start RPC calls to app-server.
    /// Allocation: PromptRunParams clone payloads (cwd/model/sandbox/attachments). Complexity: O(n), n = attachment count + prompt length.
    pub async fn ask(&self, prompt: impl Into<String>) -> Result<PromptRunResult, PromptRunError> {
        ensure_session_open_for_prompt(self.is_closed())?;
        self.runtime
            .run_prompt_in_thread_with_hooks(
                &self.thread_id,
                session_prompt_params(&self.config, prompt),
                Some(&self.config.hooks),
            )
            .await
    }

    /// Continue this session with one prompt while overriding selected turn options.
    /// Side effects: sends thread/resume + turn/start RPC calls to app-server.
    /// Allocation: depends on caller-provided params. Complexity: O(1) wrapper.
    pub async fn ask_with(
        &self,
        params: PromptRunParams,
    ) -> Result<PromptRunResult, PromptRunError> {
        ensure_session_open_for_prompt(self.is_closed())?;
        self.runtime
            .run_prompt_in_thread_with_hooks(&self.thread_id, params, Some(&self.config.hooks))
            .await
    }

    /// Continue this session with one prompt using one explicit profile override.
    /// Side effects: sends thread/resume + turn/start RPC calls to app-server.
    /// Allocation: moves profile-owned Strings/vectors + one prompt String. Complexity: O(n), n = attachment count + field sizes.
    pub async fn ask_with_profile(
        &self,
        prompt: impl Into<String>,
        profile: RunProfile,
    ) -> Result<PromptRunResult, PromptRunError> {
        ensure_session_open_for_prompt(self.is_closed())?;
        let merged_hooks = merge_hook_configs(&self.config.hooks, &profile.hooks);
        self.runtime
            .run_prompt_in_thread_with_hooks(
                &self.thread_id,
                profile_to_prompt_params(self.config.cwd.clone(), prompt, profile),
                Some(&merged_hooks),
            )
            .await
    }

    /// Return current session default profile snapshot.
    /// Allocation: clones Strings/attachments. Complexity: O(n), n = attachment count + string sizes.
    pub fn profile(&self) -> RunProfile {
        self.config.profile()
    }

    /// Interrupt one in-flight turn in this session.
    /// Side effects: sends turn/interrupt RPC call to app-server.
    /// Allocation: one small JSON payload in runtime layer. Complexity: O(1).
    pub async fn interrupt_turn(&self, turn_id: &str) -> Result<(), RpcError> {
        ensure_session_open_for_rpc(self.is_closed())?;
        self.runtime.turn_interrupt(&self.thread_id, turn_id).await
    }

    /// Archive this session on server side.
    /// Side effects: sends thread/archive RPC call to app-server.
    /// Allocation: one small JSON payload in runtime layer. Complexity: O(1).
    pub async fn close(&self) -> Result<(), RpcError> {
        {
            let guard = self.close_result.lock().await;
            if let Some(result) = guard.as_ref() {
                return result.clone();
            }
        }

        self.closed.store(true, Ordering::Release);
        let result = self.runtime.thread_archive(&self.thread_id).await;
        let mut guard = self.close_result.lock().await;
        *guard = Some(result.clone());
        result
    }
}

pub(super) fn ensure_session_open_for_prompt(closed: bool) -> Result<(), PromptRunError> {
    if closed {
        return Err(PromptRunError::Rpc(RpcError::InvalidRequest(
            "session is closed".to_owned(),
        )));
    }
    Ok(())
}

pub(super) fn ensure_session_open_for_rpc(closed: bool) -> Result<(), RpcError> {
    if closed {
        return Err(RpcError::InvalidRequest("session is closed".to_owned()));
    }
    Ok(())
}
