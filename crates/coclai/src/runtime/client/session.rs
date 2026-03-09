use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::runtime::api::{PromptRunError, PromptRunParams, PromptRunResult};
use crate::runtime::core::Runtime;
use crate::runtime::errors::RpcError;
use crate::runtime::hooks::merge_hook_configs;

use super::profile::{profile_to_prompt_params_with_hooks, session_prompt_params};
use super::{RunProfile, SessionConfig};

#[derive(Clone)]
pub struct Session {
    runtime: Runtime,
    pub thread_id: String,
    pub config: SessionConfig,
    tool_use_loop_started: Arc<AtomicBool>,
    closed: Arc<AtomicBool>,
    close_result: Arc<Mutex<Option<Result<(), RpcError>>>>,
}

impl Session {
    pub(super) fn new(
        runtime: Runtime,
        thread_id: String,
        config: SessionConfig,
        tool_use_loop_started: Arc<AtomicBool>,
    ) -> Self {
        Self {
            runtime,
            thread_id,
            config,
            tool_use_loop_started,
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
    /// Side effects: sends turn/start RPC calls on one already-loaded thread.
    /// Allocation: PromptRunParams clone payloads (cwd/model/sandbox/attachments). Complexity: O(n), n = attachment count + prompt length.
    pub async fn ask(&self, prompt: impl Into<String>) -> Result<PromptRunResult, PromptRunError> {
        ensure_session_open_for_prompt(self.is_closed())?;
        self.ensure_tool_use_hook_loop(self.config.hooks.has_pre_tool_use_hooks());
        self.runtime
            .run_prompt_on_loaded_thread_with_hooks(
                &self.thread_id,
                session_prompt_params(&self.config, prompt),
                Some(&self.config.hooks),
            )
            .await
    }

    /// Continue this session with one prompt while overriding selected turn options.
    /// Side effects: sends turn/start RPC calls on one already-loaded thread.
    /// Allocation: depends on caller-provided params. Complexity: O(1) wrapper.
    pub async fn ask_with(
        &self,
        params: PromptRunParams,
    ) -> Result<PromptRunResult, PromptRunError> {
        ensure_session_open_for_prompt(self.is_closed())?;
        self.ensure_tool_use_hook_loop(self.config.hooks.has_pre_tool_use_hooks());
        self.runtime
            .run_prompt_on_loaded_thread_with_hooks(
                &self.thread_id,
                params,
                Some(&self.config.hooks),
            )
            .await
    }

    /// Continue this session with one prompt using one explicit profile override.
    /// Side effects: sends turn/start RPC calls on one already-loaded thread.
    /// Allocation: moves profile-owned Strings/vectors + one prompt String. Complexity: O(n), n = attachment count + field sizes.
    pub async fn ask_with_profile(
        &self,
        prompt: impl Into<String>,
        profile: RunProfile,
    ) -> Result<PromptRunResult, PromptRunError> {
        ensure_session_open_for_prompt(self.is_closed())?;
        let (params, profile_hooks) =
            profile_to_prompt_params_with_hooks(self.config.cwd.clone(), prompt, profile);
        let merged_hooks = merge_hook_configs(&self.config.hooks, &profile_hooks);
        self.ensure_tool_use_hook_loop(merged_hooks.has_pre_tool_use_hooks());
        self.runtime
            .run_prompt_on_loaded_thread_with_hooks(&self.thread_id, params, Some(&merged_hooks))
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
        let mut guard = self.close_result.lock().await;
        if let Some(result) = guard.as_ref() {
            return result.clone();
        }

        self.closed.store(true, Ordering::Release);
        let result = self.runtime.thread_archive(&self.thread_id).await;
        *guard = Some(result.clone());
        result
    }

    fn ensure_tool_use_hook_loop(&self, needs_loop: bool) {
        if !needs_loop {
            return;
        }
        if self
            .tool_use_loop_started
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            tokio::spawn(
                crate::runtime::api::tool_use_hooks::run_tool_use_approval_loop(
                    self.runtime.clone(),
                ),
            );
        }
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
