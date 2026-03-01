use std::time::Duration;

use coclai_plugin_core::HookPhase;
use tokio::sync::broadcast::error::RecvError;
use tokio::time::{timeout, Instant};

use crate::errors::{RpcError, RuntimeError};
use crate::hooks::{PreHookDecision, RuntimeHookConfig};
use crate::runtime::Runtime;
use crate::turn_output::AssistantTextCollector;

use super::flow::{
    apply_pre_hook_actions_to_prompt, build_hook_context, extract_assistant_text_from_turn,
    interrupt_turn_best_effort, result_status, HookContextInput, HookExecutionState,
    LaggedTurnTerminal, PromptMutationState,
};
use super::turn_error::{extract_turn_error_signal, PromptTurnErrorSignal};
use super::wire::{build_prompt_inputs, validate_prompt_attachments};
use super::*;

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

    async fn run_prompt_target_with_hooks(
        &self,
        thread_id: Option<&str>,
        p: PromptRunParams,
        scoped_hooks: Option<&RuntimeHookConfig>,
    ) -> Result<PromptRunResult, PromptRunError> {
        if !self.hooks_enabled_with(scoped_hooks) {
            return self
                .run_prompt_entry(thread_id, p, None, scoped_hooks)
                .await;
        }

        let (p, mut hook_state, run_cwd, run_model) = self
            .prepare_prompt_pre_run_hooks(p, thread_id, scoped_hooks)
            .await;
        let result = self
            .run_prompt_entry(thread_id, p, Some(&mut hook_state), scoped_hooks)
            .await;
        self.finalize_prompt_run_hooks(
            &mut hook_state,
            run_cwd.as_str(),
            run_model.as_deref(),
            thread_id,
            &result,
            scoped_hooks,
        )
        .await;
        self.publish_hook_report(hook_state.report);
        result
    }

    fn thread_start_params_from_prompt(p: &PromptRunParams) -> ThreadStartParams {
        ThreadStartParams {
            model: p.model.clone(),
            cwd: Some(p.cwd.clone()),
            approval_policy: Some(p.approval_policy),
            sandbox_policy: Some(p.sandbox_policy.clone()),
            privileged_escalation_approved: p.privileged_escalation_approved,
        }
    }

    async fn open_prompt_thread(
        &self,
        thread_id: Option<&str>,
        p: &PromptRunParams,
    ) -> Result<ThreadHandle, RpcError> {
        let start = Self::thread_start_params_from_prompt(p);
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
        validate_prompt_attachments(&p.cwd, &p.attachments)?;
        let effort = p.effort.unwrap_or(DEFAULT_REASONING_EFFORT);
        let thread = self.open_prompt_thread(thread_id, &p).await?;
        self.run_prompt_on_thread(thread, p, effort, hook_state, scoped_hooks)
            .await
    }

    fn turn_start_params_from_prompt(
        p: &PromptRunParams,
        effort: ReasoningEffort,
    ) -> TurnStartParams {
        TurnStartParams {
            input: build_prompt_inputs(&p.prompt, &p.attachments),
            cwd: Some(p.cwd.clone()),
            approval_policy: Some(p.approval_policy),
            sandbox_policy: Some(p.sandbox_policy.clone()),
            privileged_escalation_approved: p.privileged_escalation_approved,
            model: p.model.clone(),
            effort: Some(effort),
            summary: None,
            output_schema: None,
        }
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
        );
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
            );
            state.metadata = prompt_state.metadata;
            p.prompt = prompt_state.prompt;
            p.model = prompt_state.model;
            p.attachments = prompt_state.attachments;
        }

        let live_rx = self.subscribe_live();
        let mut post_turn_id: Option<String> = None;
        let run_result = match thread
            .turn_start(Self::turn_start_params_from_prompt(&p, effort))
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
        mut live_rx: tokio::sync::broadcast::Receiver<crate::events::Envelope>,
        thread: &ThreadHandle,
        turn_id: &str,
        timeout_duration: Duration,
    ) -> Result<String, PromptRunError> {
        let mut collector = AssistantTextCollector::new();
        let mut last_turn_error: Option<PromptTurnErrorSignal> = None;
        let mut lagged_completed_text: Option<String> = None;
        let deadline = Instant::now() + timeout_duration;
        let terminal = loop {
            let now = Instant::now();
            if now >= deadline {
                interrupt_turn_best_effort(thread, turn_id);
                break Err(PromptRunError::Timeout(timeout_duration));
            }
            let remaining = deadline.saturating_duration_since(now);

            let envelope = match timeout(remaining, live_rx.recv()).await {
                Ok(Ok(v)) => v,
                Ok(Err(RecvError::Lagged(_))) => {
                    match self
                        .read_turn_terminal_after_lag(&thread.thread_id, turn_id)
                        .await
                    {
                        Ok(Some(LaggedTurnTerminal::Completed { assistant_text })) => {
                            lagged_completed_text = assistant_text;
                            break Ok(());
                        }
                        Ok(Some(LaggedTurnTerminal::Failed { message })) => {
                            if let Some(err) = last_turn_error.clone() {
                                break Err(PromptRunError::TurnFailedWithContext(
                                    err.into_failure(PromptTurnTerminalState::Failed),
                                ));
                            }
                            if let Some(message) = message {
                                break Err(PromptRunError::TurnFailedWithContext(
                                    PromptTurnFailure {
                                        terminal_state: PromptTurnTerminalState::Failed,
                                        source_method: "thread/read".to_owned(),
                                        code: None,
                                        message,
                                    },
                                ));
                            }
                            break Err(PromptRunError::TurnFailed);
                        }
                        Ok(Some(LaggedTurnTerminal::Interrupted)) => {
                            break Err(PromptRunError::TurnInterrupted);
                        }
                        Ok(None) => continue,
                        Err(err) => break Err(PromptRunError::Rpc(err)),
                    }
                }
                Ok(Err(RecvError::Closed)) => {
                    break Err(PromptRunError::Runtime(RuntimeError::Internal(format!(
                        "live stream closed: {}",
                        RecvError::Closed
                    ))));
                }
                Err(_) => {
                    interrupt_turn_best_effort(thread, turn_id);
                    break Err(PromptRunError::Timeout(timeout_duration));
                }
            };

            if envelope.thread_id.as_deref() != Some(&thread.thread_id) {
                continue;
            }
            if envelope.turn_id.as_deref() != Some(turn_id) {
                continue;
            }

            collector.push_envelope(&envelope);
            if let Some(err) = extract_turn_error_signal(&envelope) {
                last_turn_error = Some(err);
            }

            match envelope.method.as_deref() {
                Some("turn/completed") => break Ok(()),
                Some("turn/failed") => {
                    if let Some(err) = last_turn_error.clone() {
                        break Err(PromptRunError::TurnFailedWithContext(
                            err.into_failure(PromptTurnTerminalState::Failed),
                        ));
                    }
                    break Err(PromptRunError::TurnFailed);
                }
                Some("turn/interrupted") => break Err(PromptRunError::TurnInterrupted),
                _ => {}
            }
        };

        match terminal {
            Err(err) => Err(err),
            Ok(()) => Self::finalize_prompt_turn_assistant_text(
                collector,
                lagged_completed_text,
                last_turn_error,
            ),
        }
    }

    fn finalize_prompt_turn_assistant_text(
        collector: AssistantTextCollector,
        lagged_completed_text: Option<String>,
        last_turn_error: Option<PromptTurnErrorSignal>,
    ) -> Result<String, PromptRunError> {
        let assistant_text = if let Some(snapshot_text) = lagged_completed_text {
            if snapshot_text.trim().is_empty() {
                collector.into_text()
            } else {
                snapshot_text
            }
        } else {
            collector.into_text()
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
    ) -> Result<Option<LaggedTurnTerminal>, RpcError> {
        let response = self
            .thread_read(ThreadReadParams {
                thread_id: thread_id.to_owned(),
                include_turns: Some(true),
            })
            .await?;

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
