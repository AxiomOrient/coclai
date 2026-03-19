use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::runtime::api::{PromptRunError, PromptRunParams, PromptRunResult, PromptRunStream};
use crate::runtime::core::Runtime;
use crate::runtime::errors::RpcError;
use crate::runtime::hooks::merge_hook_configs;

use super::profile::{prepared_prompt_run_from_profile, session_prepared_prompt_run};
use super::{RunProfile, SessionConfig};

const SESSION_CLOSED_MESSAGE: &str = "session is closed";

#[derive(Clone)]
pub struct Session {
    runtime: Runtime,
    pub thread_id: String,
    pub config: SessionConfig,
    state: SessionState,
}

#[derive(Clone)]
pub(super) struct SessionState {
    closed: Arc<AtomicBool>,
    close_result: Arc<Mutex<Option<Result<(), RpcError>>>>,
}

struct SessionClosePermit<'a> {
    guard: tokio::sync::MutexGuard<'a, Option<Result<(), RpcError>>>,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum SessionCloseState {
    ReturnCached(Result<(), RpcError>),
    StartClosing,
}

fn ensure_session_open(closed: bool) -> Result<(), RpcError> {
    if closed {
        return Err(RpcError::InvalidRequest(SESSION_CLOSED_MESSAGE.to_owned()));
    }
    Ok(())
}

pub(super) fn next_close_state(cached: Option<&Result<(), RpcError>>) -> SessionCloseState {
    match cached {
        Some(result) => SessionCloseState::ReturnCached(result.clone()),
        None => SessionCloseState::StartClosing,
    }
}

impl SessionState {
    pub(super) fn new() -> Self {
        Self {
            closed: Arc::new(AtomicBool::new(false)),
            close_result: Arc::new(Mutex::new(None)),
        }
    }

    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    pub(super) fn ensure_open_for_prompt(&self) -> Result<(), PromptRunError> {
        ensure_session_open(self.is_closed()).map_err(PromptRunError::Rpc)
    }

    pub(super) fn ensure_open_for_rpc(&self) -> Result<(), RpcError> {
        ensure_session_open(self.is_closed())
    }

    async fn acquire_close_permit(&self) -> SessionClosePermit<'_> {
        SessionClosePermit {
            guard: self.close_result.lock().await,
        }
    }

    pub(super) fn mark_closed(&self) {
        self.closed.store(true, Ordering::Release);
    }
}

impl SessionClosePermit<'_> {
    fn next_state(&self) -> SessionCloseState {
        next_close_state(self.guard.as_ref())
    }

    fn store_result(mut self, result: Result<(), RpcError>) -> Result<(), RpcError> {
        *self.guard = Some(result.clone());
        result
    }
}

impl Session {
    pub(super) fn new(runtime: Runtime, thread_id: String, config: SessionConfig) -> Self {
        Self {
            runtime,
            thread_id,
            config,
            state: SessionState::new(),
        }
    }

    /// Returns true when this local session handle is closed.
    /// Allocation: none. Complexity: O(1).
    pub fn is_closed(&self) -> bool {
        self.state.is_closed()
    }

    /// Continue this session with one prompt.
    /// Side effects: sends turn/start RPC calls on one already-loaded thread.
    /// Allocation: PromptRunParams clone payloads (cwd/model/sandbox/attachments). Complexity: O(n), n = attachment count + prompt length.
    pub async fn ask(&self, prompt: impl Into<String>) -> Result<PromptRunResult, PromptRunError> {
        self.state.ensure_open_for_prompt()?;
        let prepared = session_prepared_prompt_run(&self.config, prompt);
        self.runtime
            .run_prompt_on_loaded_thread_with_hooks(
                &self.thread_id,
                prepared.params,
                Some(prepared.hooks.as_ref()),
            )
            .await
    }

    /// Continue this session with one prompt and receive scoped typed turn events.
    /// Side effects: sends turn/start RPC calls on one already-loaded thread and consumes only matching live events.
    pub async fn ask_stream(
        &self,
        prompt: impl Into<String>,
    ) -> Result<PromptRunStream, PromptRunError> {
        self.state.ensure_open_for_prompt()?;
        let prepared = session_prepared_prompt_run(&self.config, prompt);
        self.runtime
            .run_prompt_on_loaded_thread_stream_with_hooks(
                &self.thread_id,
                prepared.params,
                Some(prepared.hooks.as_ref()),
            )
            .await
    }

    /// Continue this session with one prompt and wait for the scoped stream to finish.
    /// Side effects: sends turn/start RPC calls on one already-loaded thread and drains the matching turn stream to completion.
    /// Allocation: PromptRunParams clone payloads (cwd/model/sandbox/attachments). Complexity: O(n), n = attachment count + prompt length.
    pub async fn ask_wait(
        &self,
        prompt: impl Into<String>,
    ) -> Result<PromptRunResult, PromptRunError> {
        self.ask_stream(prompt).await?.finish().await
    }

    /// Continue this session with one prompt while overriding selected turn options.
    /// Side effects: sends turn/start RPC calls on one already-loaded thread.
    /// Allocation: depends on caller-provided params. Complexity: O(1) wrapper.
    pub async fn ask_with(
        &self,
        params: PromptRunParams,
    ) -> Result<PromptRunResult, PromptRunError> {
        self.state.ensure_open_for_prompt()?;
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
        self.state.ensure_open_for_prompt()?;
        let prepared = prepared_prompt_run_from_profile(self.config.cwd.clone(), prompt, profile);
        let merged_hooks = merge_hook_configs(&self.config.hooks, prepared.hooks.as_ref());
        self.runtime
            .run_prompt_on_loaded_thread_with_hooks(
                &self.thread_id,
                prepared.params,
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
        self.state.ensure_open_for_rpc()?;
        self.runtime.turn_interrupt(&self.thread_id, turn_id).await
    }

    /// Archive this session on server side.
    /// Side effects: sends thread/archive RPC call to app-server.
    /// Allocation: one small JSON payload in runtime layer. Complexity: O(1).
    pub async fn close(&self) -> Result<(), RpcError> {
        let permit = self.state.acquire_close_permit().await;
        match permit.next_state() {
            SessionCloseState::ReturnCached(result) => return result,
            SessionCloseState::StartClosing => {}
        }

        self.state.mark_closed();
        let result = self.runtime.thread_archive(&self.thread_id).await;
        permit.store_result(result)
    }
}
