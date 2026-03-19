use std::time::Duration;

use crate::plugin::{BlockReason, HookPhase};
use tokio::sync::broadcast::error::RecvError;
use tokio::time::{timeout, Instant};

use crate::runtime::core::Runtime;
use crate::runtime::errors::{RpcError, RuntimeError};
use crate::runtime::events::{
    extract_agent_message_delta, extract_turn_cancelled, extract_turn_completed,
    extract_turn_failed, extract_turn_interrupted, Envelope,
};
use crate::runtime::hooks::{PreHookDecision, RuntimeHookConfig};
use crate::runtime::rpc_contract::{methods, RpcValidationMode};
use crate::runtime::turn_lifecycle::{
    collect_turn_terminal_with_limits, interrupt_turn_best_effort_detached,
    interrupt_turn_best_effort_with_timeout, LaggedTurnTerminal, TurnCollectError,
};
use crate::runtime::turn_output::{TurnStreamCollector, TurnTerminalEvent};

use super::attachment_validation::validate_prompt_attachments;
use super::flow::{
    apply_pre_hook_actions_to_prompt, build_hook_context, extract_assistant_text_from_turn,
    result_status, HookContextInput, HookExecutionState, PromptMutationState,
};
use super::models::{PromptRunStreamState, PromptStreamCleanupState};
use super::turn_error::{extract_turn_error_signal, PromptTurnErrorSignal};
use super::wire::{
    deserialize_result, serialize_params, thread_start_params_from_prompt,
    turn_start_params_from_prompt,
};
use super::*;

const INTERRUPT_RPC_TIMEOUT: Duration = Duration::from_millis(500);

#[derive(Clone, Copy)]
enum PromptRunTarget<'a> {
    OpenOrResume(Option<&'a str>),
    Loaded(&'a str),
}

impl<'a> PromptRunTarget<'a> {
    fn hook_thread_id(self) -> Option<&'a str> {
        match self {
            Self::OpenOrResume(thread_id) => thread_id,
            Self::Loaded(thread_id) => Some(thread_id),
        }
    }
}

impl Runtime {
    /// Run one prompt with safe default policies using only cwd + prompt.
    /// Side effects: same as `run_prompt`. Allocation: params object + two Strings.
    /// Complexity: O(n), n = input string lengths + streamed turn output size.
    pub async fn run_prompt_simple(
        &self,
        cwd: impl Into<String>,
        prompt: impl Into<String>,
    ) -> Result<PromptRunResult, PromptRunError> {
        self.run_prompt(PromptRunParams::new(cwd, prompt)).await
    }

    /// Run one prompt end-to-end and return the final assistant text.
    /// Side effects: sends thread/turn RPC calls and consumes live event stream.
    /// Allocation: O(n), n = prompt length + attachment count + streamed text.
    pub async fn run_prompt(&self, p: PromptRunParams) -> Result<PromptRunResult, PromptRunError> {
        self.run_prompt_with_hooks(p, None).await
    }

    pub(crate) async fn run_prompt_with_hooks(
        &self,
        p: PromptRunParams,
        scoped_hooks: Option<&RuntimeHookConfig>,
    ) -> Result<PromptRunResult, PromptRunError> {
        self.run_prompt_target_with_hooks(None, p, scoped_hooks)
            .await
    }

    /// Continue an existing thread with one additional prompt turn.
    /// Side effects: sends thread/resume + turn/start RPC calls and consumes live event stream.
    /// Allocation: O(n), n = prompt length + attachment count + streamed text.
    pub async fn run_prompt_in_thread(
        &self,
        thread_id: &str,
        p: PromptRunParams,
    ) -> Result<PromptRunResult, PromptRunError> {
        self.run_prompt_in_thread_with_hooks(thread_id, p, None)
            .await
    }

    pub(crate) async fn run_prompt_in_thread_with_hooks(
        &self,
        thread_id: &str,
        p: PromptRunParams,
        scoped_hooks: Option<&RuntimeHookConfig>,
    ) -> Result<PromptRunResult, PromptRunError> {
        self.run_prompt_target_with_hooks(Some(thread_id), p, scoped_hooks)
            .await
    }

    pub(crate) async fn run_prompt_on_loaded_thread_with_hooks(
        &self,
        thread_id: &str,
        p: PromptRunParams,
        scoped_hooks: Option<&RuntimeHookConfig>,
    ) -> Result<PromptRunResult, PromptRunError> {
        self.run_prompt_with_hook_scaffold(PromptRunTarget::Loaded(thread_id), p, scoped_hooks)
            .await
    }

    pub(crate) async fn run_prompt_on_loaded_thread_stream_with_hooks(
        &self,
        thread_id: &str,
        p: PromptRunParams,
        scoped_hooks: Option<&RuntimeHookConfig>,
    ) -> Result<PromptRunStream, PromptRunError> {
        validate_prompt_attachments(&p.cwd, &p.attachments).await?;
        let effort = p.effort.unwrap_or(DEFAULT_REASONING_EFFORT);
        let thread = self.loaded_thread_handle(thread_id);
        self.run_prompt_on_thread_stream(thread, p, effort, scoped_hooks)
            .await
    }

    async fn run_prompt_target_with_hooks(
        &self,
        thread_id: Option<&str>,
        p: PromptRunParams,
        scoped_hooks: Option<&RuntimeHookConfig>,
    ) -> Result<PromptRunResult, PromptRunError> {
        self.run_prompt_with_hook_scaffold(
            PromptRunTarget::OpenOrResume(thread_id),
            p,
            scoped_hooks,
        )
        .await
    }

    async fn run_prompt_with_hook_scaffold(
        &self,
        target: PromptRunTarget<'_>,
        p: PromptRunParams,
        scoped_hooks: Option<&RuntimeHookConfig>,
    ) -> Result<PromptRunResult, PromptRunError> {
        if !self.hooks_enabled_with(scoped_hooks) {
            return self
                .run_prompt_target_entry_dispatch(target, p, None, scoped_hooks)
                .await;
        }

        let fallback_thread_id = target.hook_thread_id();
        let (p, mut hook_state, run_cwd, run_model) = self
            .prepare_prompt_pre_run_hooks(p, fallback_thread_id, scoped_hooks)
            .await?;
        let result = self
            .run_prompt_target_entry_dispatch(target, p, Some(&mut hook_state), scoped_hooks)
            .await;
        self.finalize_prompt_run_hooks(
            &mut hook_state,
            run_cwd.as_str(),
            run_model.as_deref(),
            fallback_thread_id,
            &result,
            scoped_hooks,
        )
        .await;
        self.publish_hook_report(hook_state.report);
        result
    }

    async fn run_prompt_target_entry_dispatch(
        &self,
        target: PromptRunTarget<'_>,
        p: PromptRunParams,
        hook_state: Option<&mut HookExecutionState>,
        scoped_hooks: Option<&RuntimeHookConfig>,
    ) -> Result<PromptRunResult, PromptRunError> {
        match target {
            PromptRunTarget::OpenOrResume(thread_id) => {
                self.run_prompt_entry(thread_id, p, hook_state, scoped_hooks)
                    .await
            }
            PromptRunTarget::Loaded(thread_id) => {
                self.run_prompt_on_loaded_thread_entry(thread_id, p, hook_state, scoped_hooks)
                    .await
            }
        }
    }

    async fn open_prompt_thread(
        &self,
        thread_id: Option<&str>,
        p: &PromptRunParams,
        scoped_hooks: Option<&RuntimeHookConfig>,
    ) -> Result<ThreadHandle, RpcError> {
        let mut start = thread_start_params_from_prompt(p);
        if self.has_pre_tool_use_hooks_with(scoped_hooks)
            && matches!(start.approval_policy, None | Some(ApprovalPolicy::Never))
        {
            start.approval_policy = Some(ApprovalPolicy::Untrusted);
        }
        match thread_id {
            Some(existing_thread_id) => self.thread_resume_raw(existing_thread_id, start).await,
            None => self.thread_start_raw(start).await,
        }
    }

    async fn run_prompt_entry(
        &self,
        thread_id: Option<&str>,
        p: PromptRunParams,
        hook_state: Option<&mut HookExecutionState>,
        scoped_hooks: Option<&RuntimeHookConfig>,
    ) -> Result<PromptRunResult, PromptRunError> {
        validate_prompt_attachments(&p.cwd, &p.attachments).await?;
        let effort = p.effort.unwrap_or(DEFAULT_REASONING_EFFORT);
        let thread = self.open_prompt_thread(thread_id, &p, scoped_hooks).await?;
        self.run_prompt_on_thread(thread, p, effort, hook_state, scoped_hooks)
            .await
    }

    async fn run_prompt_on_loaded_thread_entry(
        &self,
        thread_id: &str,
        p: PromptRunParams,
        hook_state: Option<&mut HookExecutionState>,
        scoped_hooks: Option<&RuntimeHookConfig>,
    ) -> Result<PromptRunResult, PromptRunError> {
        validate_prompt_attachments(&p.cwd, &p.attachments).await?;
        let effort = p.effort.unwrap_or(DEFAULT_REASONING_EFFORT);
        let thread = self.loaded_thread_handle(thread_id);
        self.run_prompt_on_thread(thread, p, effort, hook_state, scoped_hooks)
            .await
    }

    async fn prepare_prompt_pre_run_hooks(
        &self,
        mut p: PromptRunParams,
        thread_id: Option<&str>,
        scoped_hooks: Option<&RuntimeHookConfig>,
    ) -> Result<(PromptRunParams, HookExecutionState, String, Option<String>), PromptRunError> {
        let mut hook_state = HookExecutionState::new(self.next_hook_correlation_id());
        let mut prompt_state = PromptMutationState::from_params(&p, hook_state.metadata.clone());
        let decisions = self
            .execute_pre_hook_phase(
                &mut hook_state,
                HookPhase::PreRun,
                Some(p.cwd.as_str()),
                prompt_state.model.as_deref(),
                thread_id,
                None,
                scoped_hooks,
            )
            .await
            .map_err(PromptRunError::from_block)?;
        apply_pre_hook_actions_to_prompt(
            &mut prompt_state,
            p.cwd.as_str(),
            HookPhase::PreRun,
            decisions,
            &mut hook_state.report,
        )
        .await;
        hook_state.metadata = prompt_state.metadata.clone();
        p.prompt = prompt_state.prompt;
        p.model = prompt_state.model;
        p.attachments = prompt_state.attachments;
        let run_cwd = p.cwd.clone();
        let run_model = p.model.clone();
        Ok((p, hook_state, run_cwd, run_model))
    }

    async fn finalize_prompt_run_hooks(
        &self,
        hook_state: &mut HookExecutionState,
        run_cwd: &str,
        run_model: Option<&str>,
        fallback_thread_id: Option<&str>,
        result: &Result<PromptRunResult, PromptRunError>,
        scoped_hooks: Option<&RuntimeHookConfig>,
    ) {
        let post_thread_id = result
            .as_ref()
            .ok()
            .map(|value| value.thread_id.as_str())
            .or(fallback_thread_id);
        self.execute_post_hook_phase(
            hook_state,
            HookContextInput {
                phase: HookPhase::PostRun,
                cwd: Some(run_cwd),
                model: run_model,
                thread_id: post_thread_id,
                turn_id: None,
                main_status: Some(result_status(result)),
            },
            scoped_hooks,
        )
        .await;
    }

    async fn prepare_prompt_pre_turn_hooks(
        &self,
        thread_id: &str,
        mut p: PromptRunParams,
        hook_state: Option<&mut HookExecutionState>,
        scoped_hooks: Option<&RuntimeHookConfig>,
    ) -> Result<PromptRunParams, PromptRunError> {
        let Some(state) = hook_state else {
            return Ok(p);
        };

        let mut prompt_state = PromptMutationState::from_params(&p, state.metadata.clone());
        let decisions = self
            .execute_pre_hook_phase(
                state,
                HookPhase::PreTurn,
                Some(p.cwd.as_str()),
                prompt_state.model.as_deref(),
                Some(thread_id),
                None,
                scoped_hooks,
            )
            .await
            .map_err(PromptRunError::from_block)?;
        apply_pre_hook_actions_to_prompt(
            &mut prompt_state,
            p.cwd.as_str(),
            HookPhase::PreTurn,
            decisions,
            &mut state.report,
        )
        .await;
        state.metadata = prompt_state.metadata;
        p.prompt = prompt_state.prompt;
        p.model = prompt_state.model;
        p.attachments = prompt_state.attachments;
        Ok(p)
    }

    async fn run_prompt_on_thread(
        &self,
        thread: ThreadHandle,
        p: PromptRunParams,
        effort: ReasoningEffort,
        hook_state: Option<&mut HookExecutionState>,
        scoped_hooks: Option<&RuntimeHookConfig>,
    ) -> Result<PromptRunResult, PromptRunError> {
        let mut hook_state = hook_state;
        let p = self
            .prepare_prompt_pre_turn_hooks(
                thread.thread_id.as_str(),
                p,
                hook_state.as_deref_mut(),
                scoped_hooks,
            )
            .await?;

        self.register_thread_scoped_pre_tool_use_hooks(&thread.thread_id, scoped_hooks);
        let live_rx = self.subscribe_live();
        let mut post_turn_id: Option<String> = None;
        let run_result = match thread
            .turn_start(turn_start_params_from_prompt(&p, effort))
            .await
            .map_err(PromptRunError::Rpc)
        {
            Ok(turn) => {
                post_turn_id = Some(turn.turn_id.clone());
                self.collect_prompt_turn_assistant_text(live_rx, &thread, &turn.turn_id, p.timeout)
                    .await
                    .map(|assistant_text| PromptRunResult {
                        thread_id: thread.thread_id.clone(),
                        turn_id: turn.turn_id,
                        assistant_text,
                    })
            }
            Err(err) => Err(err),
        };

        if let Some(state) = hook_state {
            self.execute_post_hook_phase(
                state,
                HookContextInput {
                    phase: HookPhase::PostTurn,
                    cwd: Some(p.cwd.as_str()),
                    model: p.model.as_deref(),
                    thread_id: Some(thread.thread_id.as_str()),
                    turn_id: post_turn_id.as_deref(),
                    main_status: Some(result_status(&run_result)),
                },
                scoped_hooks,
            )
            .await;
        }

        self.clear_thread_scoped_pre_tool_use_hooks(&thread.thread_id);
        run_result
    }

    async fn run_prompt_on_thread_stream(
        &self,
        thread: ThreadHandle,
        p: PromptRunParams,
        effort: ReasoningEffort,
        scoped_hooks: Option<&RuntimeHookConfig>,
    ) -> Result<PromptRunStream, PromptRunError> {
        let mut hook_state = if self.hooks_enabled_with(scoped_hooks) {
            Some(HookExecutionState::new(self.next_hook_correlation_id()))
        } else {
            None
        };
        let p = self
            .prepare_prompt_pre_turn_hooks(
                thread.thread_id.as_str(),
                p,
                hook_state.as_mut(),
                scoped_hooks,
            )
            .await?;

        self.register_thread_scoped_pre_tool_use_hooks(&thread.thread_id, scoped_hooks);
        let live_rx = self.subscribe_live();
        let timeout_duration = p.timeout;
        let run_cwd = p.cwd.clone();
        let run_model = p.model.clone();

        let turn = match thread
            .turn_start(turn_start_params_from_prompt(&p, effort))
            .await
            .map_err(PromptRunError::Rpc)
        {
            Ok(turn) => turn,
            Err(err) => {
                if let Some(state) = hook_state.as_mut() {
                    self.execute_post_hook_phase(
                        state,
                        HookContextInput {
                            phase: HookPhase::PostTurn,
                            cwd: Some(run_cwd.as_str()),
                            model: run_model.as_deref(),
                            thread_id: Some(thread.thread_id.as_str()),
                            turn_id: None,
                            main_status: Some("error"),
                        },
                        scoped_hooks,
                    )
                    .await;
                    self.publish_hook_report(state.report.clone());
                }
                self.clear_thread_scoped_pre_tool_use_hooks(&thread.thread_id);
                return Err(err);
            }
        };
        let cleanup = PromptStreamCleanupState {
            run_cwd,
            run_model,
            scoped_hooks: scoped_hooks.cloned(),
            hook_state,
            cleaned_up: false,
        };

        Ok(PromptRunStream {
            runtime: self.clone(),
            thread_id: thread.thread_id.clone(),
            turn_id: turn.turn_id.clone(),
            live_rx,
            stream: TurnStreamCollector::new(&thread.thread_id, &turn.turn_id),
            state: PromptRunStreamState {
                last_turn_error: None,
                lagged_terminal: None,
                final_result: None,
            },
            deadline: Instant::now() + timeout_duration,
            timeout: timeout_duration,
            cleanup,
        })
    }

    async fn collect_prompt_turn_assistant_text(
        &self,
        mut live_rx: tokio::sync::broadcast::Receiver<crate::runtime::events::Envelope>,
        thread: &ThreadHandle,
        turn_id: &str,
        timeout_duration: Duration,
    ) -> Result<String, PromptRunError> {
        let mut stream = TurnStreamCollector::new(&thread.thread_id, turn_id);
        let mut last_turn_error: Option<PromptTurnErrorSignal> = None;
        let collected = collect_turn_terminal_with_limits(
            &mut live_rx,
            &mut stream,
            usize::MAX,
            timeout_duration,
            |envelope| {
                if let Some(err) = extract_turn_error_signal(envelope) {
                    last_turn_error = Some(err);
                }
                Ok::<(), RpcError>(())
            },
            |lag_probe_budget| async move {
                self.read_turn_terminal_after_lag(&thread.thread_id, turn_id, lag_probe_budget)
                    .await
            },
        )
        .await;

        let (terminal, lagged_terminal) = match collected {
            Ok(result) => result,
            Err(TurnCollectError::Timeout) => {
                interrupt_turn_best_effort_detached(
                    thread.runtime().clone(),
                    thread.thread_id.clone(),
                    turn_id.to_owned(),
                    INTERRUPT_RPC_TIMEOUT,
                );
                return Err(PromptRunError::Timeout(timeout_duration));
            }
            Err(TurnCollectError::StreamClosed) => {
                return Err(PromptRunError::Runtime(RuntimeError::Internal(format!(
                    "live stream closed: {}",
                    RecvError::Closed
                ))));
            }
            Err(TurnCollectError::EventBudgetExceeded) => {
                return Err(PromptRunError::Runtime(RuntimeError::Internal(
                    "turn event budget exhausted while collecting assistant output".to_owned(),
                )));
            }
            Err(TurnCollectError::TargetEnvelope(err)) => return Err(PromptRunError::Rpc(err)),
            Err(TurnCollectError::LagProbe(RpcError::Timeout)) => {
                interrupt_turn_best_effort_detached(
                    thread.runtime().clone(),
                    thread.thread_id.clone(),
                    turn_id.to_owned(),
                    INTERRUPT_RPC_TIMEOUT,
                );
                return Err(PromptRunError::Timeout(timeout_duration));
            }
            Err(TurnCollectError::LagProbe(err)) => return Err(PromptRunError::Rpc(err)),
        };

        Self::resolve_prompt_turn_assistant_text(
            terminal,
            stream.into_assistant_text(),
            lagged_terminal.as_ref(),
            last_turn_error,
        )
    }

    fn resolve_prompt_turn_assistant_text(
        terminal: TurnTerminalEvent,
        collected_assistant_text: String,
        lagged_terminal: Option<&LaggedTurnTerminal>,
        last_turn_error: Option<PromptTurnErrorSignal>,
    ) -> Result<String, PromptRunError> {
        match terminal {
            TurnTerminalEvent::Completed => Self::finalize_prompt_turn_assistant_text(
                collected_assistant_text,
                lagged_completed_text(lagged_terminal),
                last_turn_error,
            ),
            TurnTerminalEvent::Failed => prompt_turn_failed_error(last_turn_error, lagged_terminal),
            TurnTerminalEvent::Interrupted | TurnTerminalEvent::Cancelled => {
                Err(PromptRunError::TurnInterrupted)
            }
        }
    }

    fn finalize_prompt_turn_assistant_text(
        collected_assistant_text: String,
        lagged_completed_text: Option<String>,
        last_turn_error: Option<PromptTurnErrorSignal>,
    ) -> Result<String, PromptRunError> {
        let assistant_text = if let Some(snapshot_text) = lagged_completed_text {
            if snapshot_text.trim().is_empty() {
                collected_assistant_text
            } else {
                snapshot_text
            }
        } else {
            collected_assistant_text
        };
        let assistant_text = assistant_text.trim().to_owned();
        if assistant_text.is_empty() {
            if let Some(err) = last_turn_error {
                Err(PromptRunError::TurnCompletedWithoutAssistantText(
                    err.into_failure(PromptTurnTerminalState::CompletedWithoutAssistantText),
                ))
            } else {
                Err(PromptRunError::EmptyAssistantText)
            }
        } else {
            Ok(assistant_text)
        }
    }

    async fn read_turn_terminal_after_lag(
        &self,
        thread_id: &str,
        turn_id: &str,
        timeout_duration: Duration,
    ) -> Result<Option<LaggedTurnTerminal>, RpcError> {
        let params = serialize_params(
            methods::THREAD_READ,
            &ThreadReadParams {
                thread_id: thread_id.to_owned(),
                include_turns: Some(true),
            },
        )?;
        let response = self
            .call_validated_with_mode_and_timeout(
                methods::THREAD_READ,
                params,
                RpcValidationMode::KnownMethods,
                timeout_duration,
            )
            .await?;
        let response: ThreadReadResponse = deserialize_result(methods::THREAD_READ, response)?;

        let Some(turn) = response.thread.turns.iter().find(|turn| turn.id == turn_id) else {
            return Ok(None);
        };

        Ok(lagged_terminal_from_turn(turn))
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn execute_pre_hook_phase(
        &self,
        hook_state: &mut HookExecutionState,
        phase: HookPhase,
        cwd: Option<&str>,
        model: Option<&str>,
        thread_id: Option<&str>,
        turn_id: Option<&str>,
        scoped_hooks: Option<&RuntimeHookConfig>,
    ) -> Result<Vec<PreHookDecision>, BlockReason> {
        let ctx = build_hook_context(
            hook_state.correlation_id.as_str(),
            &hook_state.metadata,
            HookContextInput {
                phase,
                cwd,
                model,
                thread_id,
                turn_id,
                main_status: None,
            },
        );
        self.run_pre_hooks_with(&ctx, &mut hook_state.report, scoped_hooks)
            .await
    }

    pub(super) async fn execute_post_hook_phase(
        &self,
        hook_state: &mut HookExecutionState,
        input: HookContextInput<'_>,
        scoped_hooks: Option<&RuntimeHookConfig>,
    ) {
        let ctx = build_hook_context(
            hook_state.correlation_id.as_str(),
            &hook_state.metadata,
            input,
        );
        self.run_post_hooks_with(&ctx, &mut hook_state.report, scoped_hooks)
            .await;
    }
}

impl PromptRunStream {
    /// Borrow target thread id for this scoped stream.
    pub fn thread_id(&self) -> &str {
        self.thread_id.as_str()
    }

    /// Borrow target turn id for this scoped stream.
    pub fn turn_id(&self) -> &str {
        self.turn_id.as_str()
    }

    /// Receive the next typed event for the target turn.
    pub async fn recv(&mut self) -> Result<Option<PromptRunStreamEvent>, PromptRunError> {
        if self.state.final_result.is_some() {
            return Ok(None);
        }

        loop {
            let now = Instant::now();
            if now >= self.deadline {
                return Err(self.timeout_with_interrupt().await);
            }
            let remaining = self.deadline.saturating_duration_since(now);

            let envelope = match timeout(remaining, self.live_rx.recv()).await {
                Ok(Ok(envelope)) => envelope,
                Ok(Err(RecvError::Lagged(_))) => {
                    let lag_probe_budget = self.deadline.saturating_duration_since(Instant::now());
                    if lag_probe_budget.is_zero() {
                        return Err(self.timeout_with_interrupt().await);
                    }

                    match self
                        .runtime
                        .read_turn_terminal_after_lag(
                            &self.thread_id,
                            &self.turn_id,
                            lag_probe_budget,
                        )
                        .await
                    {
                        Ok(Some(snapshot)) => {
                            self.state.lagged_terminal = Some(snapshot.clone());
                            let observation =
                                observe_lagged_terminal(&self.thread_id, &self.turn_id, &snapshot);
                            return Ok(self.apply_observation(observation).await);
                        }
                        Ok(None) => continue,
                        Err(RpcError::Timeout) => return Err(self.timeout_with_interrupt().await),
                        Err(err) => return Err(self.fail(PromptRunError::Rpc(err)).await),
                    }
                }
                Ok(Err(RecvError::Closed)) => {
                    return Err(self
                        .fail(PromptRunError::Runtime(RuntimeError::Internal(format!(
                            "live stream closed: {}",
                            RecvError::Closed
                        ))))
                        .await);
                }
                Err(_) => return Err(self.timeout_with_interrupt().await),
            };

            if !self.stream.is_target_envelope(&envelope) {
                continue;
            }

            let terminal = self.stream.push_envelope(&envelope);
            let observation = observe_target_envelope(&envelope, terminal);
            let next = self.apply_observation(observation).await;
            if next.is_some() {
                return Ok(next);
            }
            if self.state.final_result.is_some() {
                return Ok(None);
            }
        }
    }

    /// Drain the stream to its terminal result.
    pub async fn finish(mut self) -> Result<PromptRunResult, PromptRunError> {
        while self.state.final_result.is_none() {
            if self.recv().await?.is_none() {
                break;
            }
        }

        match self.state.final_result.take() {
            Some(result) => result,
            None => Err(PromptRunError::Runtime(RuntimeError::Internal(
                "prompt stream finished without terminal result".to_owned(),
            ))),
        }
    }

    async fn complete(&mut self, result: Result<PromptRunResult, PromptRunError>) {
        self.cleanup(stream_result_status(&result)).await;
        self.state.final_result = Some(result);
    }

    async fn timeout_with_interrupt(&mut self) -> PromptRunError {
        self.interrupt_best_effort();
        self.fail(PromptRunError::Timeout(self.timeout)).await
    }

    async fn fail(&mut self, err: PromptRunError) -> PromptRunError {
        self.cleanup("error").await;
        self.state.final_result = Some(Err(err.clone()));
        err
    }

    async fn cleanup(&mut self, main_status: &'static str) {
        let Some(plan) = self.take_cleanup_plan(main_status, false) else {
            return;
        };
        run_cleanup_plan(&self.runtime, plan).await;
    }

    fn detach_cleanup(&mut self, main_status: &'static str) {
        let Some(plan) = self.take_cleanup_plan(main_status, true) else {
            return;
        };

        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            let runtime = self.runtime.clone();
            handle.spawn(async move {
                run_cleanup_plan(&runtime, plan).await;
            });
        } else {
            self.runtime
                .clear_thread_scoped_pre_tool_use_hooks(&self.thread_id);
        }
    }

    fn interrupt_best_effort(&self) {
        interrupt_turn_best_effort_detached(
            self.runtime.clone(),
            self.thread_id.clone(),
            self.turn_id.clone(),
            INTERRUPT_RPC_TIMEOUT,
        );
    }

    fn take_cleanup_plan(
        &mut self,
        main_status: &'static str,
        send_interrupt: bool,
    ) -> Option<PromptStreamCleanupPlan> {
        self.cleanup.take_plan(
            self.thread_id.clone(),
            self.turn_id.clone(),
            main_status,
            send_interrupt,
        )
    }

    async fn apply_observation(
        &mut self,
        observation: PromptStreamObservation,
    ) -> Option<PromptRunStreamEvent> {
        let transition = reduce_prompt_stream_observation(
            &mut self.state,
            self.thread_id.as_str(),
            self.turn_id.as_str(),
            self.stream.clone().into_assistant_text(),
            observation,
        );
        if let Some(result) = transition.terminal_result {
            self.complete(result).await;
        }
        transition.event
    }
}

fn lagged_completed_text(lagged_terminal: Option<&LaggedTurnTerminal>) -> Option<String> {
    match lagged_terminal {
        Some(LaggedTurnTerminal::Completed { assistant_text }) => assistant_text.clone(),
        _ => None,
    }
}

fn prompt_turn_failed_error(
    last_turn_error: Option<PromptTurnErrorSignal>,
    lagged_terminal: Option<&LaggedTurnTerminal>,
) -> Result<String, PromptRunError> {
    if let Some(err) = last_turn_error {
        Err(PromptRunError::TurnFailedWithContext(
            err.into_failure(PromptTurnTerminalState::Failed),
        ))
    } else if let Some(LaggedTurnTerminal::Failed { message }) = lagged_terminal {
        if let Some(message) = message.clone() {
            Err(PromptRunError::TurnFailedWithContext(PromptTurnFailure {
                terminal_state: PromptTurnTerminalState::Failed,
                source_method: "thread/read".to_owned(),
                code: None,
                message,
            }))
        } else {
            Err(PromptRunError::TurnFailed)
        }
    } else {
        Err(PromptRunError::TurnFailed)
    }
}

impl Drop for PromptRunStream {
    fn drop(&mut self) {
        if !self.cleanup.cleaned_up {
            self.detach_cleanup("error");
        }
    }
}

fn envelope_to_stream_event(envelope: &Envelope) -> Option<PromptRunStreamEvent> {
    extract_agent_message_delta(envelope)
        .map(PromptRunStreamEvent::AgentMessageDelta)
        .or_else(|| extract_turn_completed(envelope).map(PromptRunStreamEvent::TurnCompleted))
        .or_else(|| extract_turn_failed(envelope).map(PromptRunStreamEvent::TurnFailed))
        .or_else(|| extract_turn_interrupted(envelope).map(PromptRunStreamEvent::TurnInterrupted))
        .or_else(|| extract_turn_cancelled(envelope).map(PromptRunStreamEvent::TurnCancelled))
}

fn lagged_terminal_to_stream_event(
    thread_id: &str,
    turn_id: &str,
    terminal: &LaggedTurnTerminal,
) -> Option<PromptRunStreamEvent> {
    match terminal {
        LaggedTurnTerminal::Completed { assistant_text } => {
            Some(PromptRunStreamEvent::TurnCompleted(
                crate::runtime::events::TurnCompletedNotification {
                    thread_id: thread_id.to_owned(),
                    turn_id: turn_id.to_owned(),
                    text: assistant_text.clone(),
                },
            ))
        }
        LaggedTurnTerminal::Failed { message } => Some(PromptRunStreamEvent::TurnFailed(
            crate::runtime::events::TurnFailedNotification {
                thread_id: thread_id.to_owned(),
                turn_id: turn_id.to_owned(),
                code: None,
                message: message.clone(),
            },
        )),
        LaggedTurnTerminal::Cancelled => Some(PromptRunStreamEvent::TurnCancelled(
            crate::runtime::events::TurnCancelledNotification {
                thread_id: thread_id.to_owned(),
                turn_id: turn_id.to_owned(),
            },
        )),
        LaggedTurnTerminal::Interrupted => Some(PromptRunStreamEvent::TurnInterrupted(
            crate::runtime::events::TurnInterruptedNotification {
                thread_id: thread_id.to_owned(),
                turn_id: turn_id.to_owned(),
            },
        )),
    }
}

fn observe_target_envelope(
    envelope: &Envelope,
    terminal: Option<TurnTerminalEvent>,
) -> PromptStreamObservation {
    PromptStreamObservation {
        event: envelope_to_stream_event(envelope),
        terminal,
        turn_error: extract_turn_error_signal(envelope),
    }
}

fn observe_lagged_terminal(
    thread_id: &str,
    turn_id: &str,
    terminal: &LaggedTurnTerminal,
) -> PromptStreamObservation {
    PromptStreamObservation {
        event: lagged_terminal_to_stream_event(thread_id, turn_id, terminal),
        terminal: Some(terminal.as_terminal_event()),
        turn_error: None,
    }
}

fn lagged_terminal_from_turn(turn: &ThreadTurnView) -> Option<LaggedTurnTerminal> {
    match turn.status {
        ThreadTurnStatus::Completed => Some(LaggedTurnTerminal::Completed {
            assistant_text: extract_assistant_text_from_turn(turn),
        }),
        ThreadTurnStatus::Failed => Some(LaggedTurnTerminal::Failed {
            message: turn.error.as_ref().map(|error| error.message.clone()),
        }),
        ThreadTurnStatus::Cancelled => Some(LaggedTurnTerminal::Cancelled),
        ThreadTurnStatus::Interrupted => Some(LaggedTurnTerminal::Interrupted),
        ThreadTurnStatus::InProgress => None,
    }
}

fn stream_result_status(result: &Result<PromptRunResult, PromptRunError>) -> &'static str {
    if result.is_ok() {
        "ok"
    } else {
        "error"
    }
}

struct PromptStreamCleanupPlan {
    thread_id: String,
    turn_id: String,
    run_cwd: String,
    run_model: Option<String>,
    scoped_hooks: Option<RuntimeHookConfig>,
    hook_state: Option<HookExecutionState>,
    main_status: &'static str,
    send_interrupt: bool,
}

struct PromptStreamObservation {
    event: Option<PromptRunStreamEvent>,
    terminal: Option<TurnTerminalEvent>,
    turn_error: Option<PromptTurnErrorSignal>,
}

struct PromptStreamTransition {
    event: Option<PromptRunStreamEvent>,
    terminal_result: Option<Result<PromptRunResult, PromptRunError>>,
}

fn reduce_prompt_stream_observation(
    state: &mut PromptRunStreamState,
    thread_id: &str,
    turn_id: &str,
    collected_assistant_text: String,
    observation: PromptStreamObservation,
) -> PromptStreamTransition {
    if let Some(err) = observation.turn_error {
        state.last_turn_error = Some(err);
    }

    let terminal_result = observation.terminal.map(|terminal| {
        build_prompt_run_result(
            thread_id,
            turn_id,
            collected_assistant_text,
            state.lagged_terminal.as_ref(),
            state.last_turn_error.clone(),
            terminal,
        )
    });

    PromptStreamTransition {
        event: observation.event,
        terminal_result,
    }
}

fn build_prompt_run_result(
    thread_id: &str,
    turn_id: &str,
    collected_assistant_text: String,
    lagged_terminal: Option<&LaggedTurnTerminal>,
    last_turn_error: Option<PromptTurnErrorSignal>,
    terminal: TurnTerminalEvent,
) -> Result<PromptRunResult, PromptRunError> {
    Runtime::resolve_prompt_turn_assistant_text(
        terminal,
        collected_assistant_text,
        lagged_terminal,
        last_turn_error,
    )
    .map(|assistant_text| PromptRunResult {
        thread_id: thread_id.to_owned(),
        turn_id: turn_id.to_owned(),
        assistant_text,
    })
}

impl PromptStreamCleanupState {
    fn take_plan(
        &mut self,
        thread_id: String,
        turn_id: String,
        main_status: &'static str,
        send_interrupt: bool,
    ) -> Option<PromptStreamCleanupPlan> {
        if self.cleaned_up {
            return None;
        }

        self.cleaned_up = true;
        Some(PromptStreamCleanupPlan {
            thread_id,
            turn_id,
            run_cwd: self.run_cwd.clone(),
            run_model: self.run_model.clone(),
            scoped_hooks: self.scoped_hooks.clone(),
            hook_state: self.hook_state.take(),
            main_status,
            send_interrupt,
        })
    }
}

async fn run_cleanup_plan(runtime: &Runtime, mut plan: PromptStreamCleanupPlan) {
    if let Some(state) = plan.hook_state.as_mut() {
        runtime
            .execute_post_hook_phase(
                state,
                HookContextInput {
                    phase: HookPhase::PostTurn,
                    cwd: Some(plan.run_cwd.as_str()),
                    model: plan.run_model.as_deref(),
                    thread_id: Some(plan.thread_id.as_str()),
                    turn_id: Some(plan.turn_id.as_str()),
                    main_status: Some(plan.main_status),
                },
                plan.scoped_hooks.as_ref(),
            )
            .await;
        runtime.publish_hook_report(state.report.clone());
    }

    if plan.send_interrupt {
        interrupt_turn_best_effort_with_timeout(
            runtime,
            plan.thread_id.as_str(),
            plan.turn_id.as_str(),
            INTERRUPT_RPC_TIMEOUT,
        )
        .await;
    }

    runtime.clear_thread_scoped_pre_tool_use_hooks(&plan.thread_id);
}
