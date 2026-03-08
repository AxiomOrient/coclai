use std::time::Duration;

use crate::plugin::HookPhase;
use tokio::sync::broadcast::error::RecvError;

use crate::runtime::core::Runtime;
use crate::runtime::errors::{RpcError, RuntimeError};
use crate::runtime::hooks::{PreHookDecision, RuntimeHookConfig};
use crate::runtime::rpc_contract::{methods, RpcValidationMode};
use crate::runtime::turn_lifecycle::{
    collect_turn_terminal_with_limits, interrupt_turn_best_effort_detached, LaggedTurnTerminal,
    TurnCollectError,
};
use crate::runtime::turn_output::{TurnStreamCollector, TurnTerminalEvent};

use super::attachment_validation::validate_prompt_attachments;
use super::flow::{
    apply_pre_hook_actions_to_prompt, build_hook_context, extract_assistant_text_from_turn,
    result_status, HookContextInput, HookExecutionState, PromptMutationState,
};
use super::turn_error::{extract_turn_error_signal, PromptTurnErrorSignal};
use super::wire::{
    deserialize_result, serialize_params, thread_start_params_from_prompt,
    turn_start_params_from_prompt,
};
use super::*;

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
            .await;
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
    ) -> Result<ThreadHandle, RpcError> {
        let start = thread_start_params_from_prompt(p);
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
        let thread = self.open_prompt_thread(thread_id, &p).await?;
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
    ) -> (PromptRunParams, HookExecutionState, String, Option<String>) {
        let mut hook_state = HookExecutionState::new(self.next_hook_correlation_id());
        let mut prompt_state = PromptMutationState::from_params(&p, hook_state.metadata.clone());
        let decisions = self
            .execute_pre_hook_phase(
                &mut hook_state,
                HookPhase::PreRun,
                Some(prompt_state.prompt.as_str()),
                prompt_state.model.as_deref(),
                thread_id,
                None,
                scoped_hooks,
            )
            .await;
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
        (p, hook_state, run_cwd, run_model)
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

    async fn run_prompt_on_thread(
        &self,
        thread: ThreadHandle,
        p: PromptRunParams,
        effort: ReasoningEffort,
        hook_state: Option<&mut HookExecutionState>,
        scoped_hooks: Option<&RuntimeHookConfig>,
    ) -> Result<PromptRunResult, PromptRunError> {
        let mut hook_state = hook_state;
        let mut p = p;
        if let Some(state) = hook_state.as_deref_mut() {
            let mut prompt_state = PromptMutationState::from_params(&p, state.metadata.clone());
            let decisions = self
                .execute_pre_hook_phase(
                    state,
                    HookPhase::PreTurn,
                    Some(prompt_state.prompt.as_str()),
                    prompt_state.model.as_deref(),
                    Some(thread.thread_id.as_str()),
                    None,
                    scoped_hooks,
                )
                .await;
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
        }

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

        run_result
    }

    async fn collect_prompt_turn_assistant_text(
        &self,
        mut live_rx: tokio::sync::broadcast::Receiver<crate::runtime::events::Envelope>,
        thread: &ThreadHandle,
        turn_id: &str,
        timeout_duration: Duration,
    ) -> Result<String, PromptRunError> {
        const INTERRUPT_RPC_TIMEOUT: Duration = Duration::from_millis(500);

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

        let lagged_completed_text = match lagged_terminal.as_ref() {
            Some(LaggedTurnTerminal::Completed { assistant_text }) => assistant_text.clone(),
            _ => None,
        };

        match terminal {
            TurnTerminalEvent::Completed => Self::finalize_prompt_turn_assistant_text(
                stream.into_assistant_text(),
                lagged_completed_text,
                last_turn_error,
            ),
            TurnTerminalEvent::Failed => {
                if let Some(err) = last_turn_error {
                    Err(PromptRunError::TurnFailedWithContext(
                        err.into_failure(PromptTurnTerminalState::Failed),
                    ))
                } else if let Some(LaggedTurnTerminal::Failed { message }) =
                    lagged_terminal.as_ref()
                {
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

        let terminal = match turn.status {
            ThreadTurnStatus::Completed => Some(LaggedTurnTerminal::Completed {
                assistant_text: extract_assistant_text_from_turn(turn),
            }),
            ThreadTurnStatus::Failed => Some(LaggedTurnTerminal::Failed {
                message: turn.error.as_ref().map(|error| error.message.clone()),
            }),
            ThreadTurnStatus::Interrupted => Some(LaggedTurnTerminal::Interrupted),
            ThreadTurnStatus::InProgress => None,
        };
        Ok(terminal)
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
    ) -> Vec<PreHookDecision> {
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
