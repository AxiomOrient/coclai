use std::time::Duration;

use crate::plugin::HookPhase;
use serde_json::{Map, Value};

use crate::runtime::core::Runtime;
use crate::runtime::errors::RpcError;
use crate::runtime::hooks::RuntimeHookConfig;
use crate::runtime::rpc_contract::{methods, RpcValidationMode};
use crate::runtime::turn_output::{parse_thread_id, parse_turn_id};

use super::flow::{
    apply_pre_hook_actions_to_session, result_status, HookContextInput, HookExecutionState,
    SessionMutationState,
};
use super::wire::{
    deserialize_result, input_item_to_wire, serialize_params, thread_overrides_to_wire,
    turn_start_params_to_wire, validate_turn_start_security,
};
use super::*;

impl ThreadHandle {
    pub fn runtime(&self) -> &crate::runtime::core::Runtime {
        &self.runtime
    }

    pub async fn turn_start(&self, p: TurnStartParams) -> Result<TurnHandle, RpcError> {
        ensure_turn_input_not_empty(&p.input)?;
        validate_turn_start_security(&p)?;

        let response = self
            .runtime
            .call_validated(
                methods::TURN_START,
                turn_start_params_to_wire(&self.thread_id, &p),
            )
            .await?;

        let turn_id = parse_turn_id(&response).ok_or_else(|| {
            RpcError::InvalidRequest(format!("turn/start missing turn id in result: {response}"))
        })?;

        Ok(TurnHandle {
            turn_id,
            thread_id: self.thread_id.clone(),
        })
    }

    /// Start a follow-up turn anchored to an expected previous turn id.
    /// Allocation: JSON params + input item wire objects.
    /// Complexity: O(n), n = input item count.
    pub async fn turn_steer(
        &self,
        expected_turn_id: &str,
        input: Vec<InputItem>,
    ) -> Result<super::TurnId, RpcError> {
        ensure_turn_input_not_empty(&input)?;

        let mut params = Map::<String, Value>::new();
        params.insert("threadId".to_owned(), Value::String(self.thread_id.clone()));
        params.insert(
            "expectedTurnId".to_owned(),
            Value::String(expected_turn_id.to_owned()),
        );
        params.insert(
            "input".to_owned(),
            Value::Array(input.iter().map(input_item_to_wire).collect()),
        );
        let response = self
            .runtime
            .call_validated(methods::TURN_START, Value::Object(params))
            .await?;
        parse_turn_id(&response).ok_or_else(|| {
            RpcError::InvalidRequest(format!(
                "turn/start(steer) missing turn id in result: {response}"
            ))
        })
    }

    pub async fn turn_interrupt(&self, turn_id: &str) -> Result<(), RpcError> {
        self.runtime.turn_interrupt(&self.thread_id, turn_id).await
    }
}

impl Runtime {
    pub(crate) fn loaded_thread_handle(&self, thread_id: &str) -> ThreadHandle {
        ThreadHandle {
            thread_id: thread_id.to_owned(),
            runtime: self.clone(),
        }
    }

    pub async fn thread_start(&self, p: ThreadStartParams) -> Result<ThreadHandle, RpcError> {
        self.thread_start_with_hooks(p, None).await
    }

    pub(crate) async fn thread_start_with_hooks(
        &self,
        p: ThreadStartParams,
        scoped_hooks: Option<&RuntimeHookConfig>,
    ) -> Result<ThreadHandle, RpcError> {
        if !self.hooks_enabled_with(scoped_hooks) {
            return self.thread_start_raw(p).await;
        }

        let (p, mut hook_state, start_cwd, start_model) = self
            .prepare_session_start_hooks(p, None, scoped_hooks)
            .await;
        let result = self.thread_start_raw(p).await;
        self.finalize_session_start_hooks(
            &mut hook_state,
            start_cwd.as_deref(),
            start_model.as_deref(),
            None,
            &result,
            scoped_hooks,
        )
        .await;
        self.publish_hook_report(hook_state.report);
        result
    }

    pub async fn thread_resume(
        &self,
        thread_id: &str,
        p: ThreadStartParams,
    ) -> Result<ThreadHandle, RpcError> {
        self.thread_resume_with_hooks(thread_id, p, None).await
    }

    pub(crate) async fn thread_resume_with_hooks(
        &self,
        thread_id: &str,
        p: ThreadStartParams,
        scoped_hooks: Option<&RuntimeHookConfig>,
    ) -> Result<ThreadHandle, RpcError> {
        if !self.hooks_enabled_with(scoped_hooks) {
            return self.thread_resume_raw(thread_id, p).await;
        }

        let (p, mut hook_state, resume_cwd, resume_model) = self
            .prepare_session_start_hooks(p, Some(thread_id), scoped_hooks)
            .await;
        let result = self.thread_resume_raw(thread_id, p).await;
        self.finalize_session_start_hooks(
            &mut hook_state,
            resume_cwd.as_deref(),
            resume_model.as_deref(),
            Some(thread_id),
            &result,
            scoped_hooks,
        )
        .await;
        self.publish_hook_report(hook_state.report);
        result
    }

    async fn prepare_session_start_hooks(
        &self,
        p: ThreadStartParams,
        thread_id: Option<&str>,
        scoped_hooks: Option<&RuntimeHookConfig>,
    ) -> (
        ThreadStartParams,
        HookExecutionState,
        Option<String>,
        Option<String>,
    ) {
        let mut hook_state = HookExecutionState::new(self.next_hook_correlation_id());
        let mut session_state =
            SessionMutationState::from_thread_start(&p, hook_state.metadata.clone());
        let decisions = self
            .execute_pre_hook_phase(
                &mut hook_state,
                HookPhase::PreSessionStart,
                p.cwd.as_deref(),
                p.model.as_deref(),
                thread_id,
                None,
                scoped_hooks,
            )
            .await;
        apply_pre_hook_actions_to_session(
            &mut session_state,
            HookPhase::PreSessionStart,
            decisions,
            &mut hook_state.report,
        );
        hook_state.metadata = session_state.metadata.clone();

        let mut p = p;
        p.model = session_state.model;
        let cwd = p.cwd.clone();
        let model = p.model.clone();

        (p, hook_state, cwd, model)
    }

    async fn finalize_session_start_hooks(
        &self,
        hook_state: &mut HookExecutionState,
        cwd: Option<&str>,
        model: Option<&str>,
        fallback_thread_id: Option<&str>,
        result: &Result<ThreadHandle, RpcError>,
        scoped_hooks: Option<&RuntimeHookConfig>,
    ) {
        let post_thread_id = result
            .as_ref()
            .ok()
            .map(|thread| thread.thread_id.as_str())
            .or(fallback_thread_id);
        self.execute_post_hook_phase(
            hook_state,
            HookContextInput {
                phase: HookPhase::PostSessionStart,
                cwd,
                model,
                thread_id: post_thread_id,
                turn_id: None,
                main_status: Some(result_status(result)),
            },
            scoped_hooks,
        )
        .await;
    }

    pub(crate) async fn thread_resume_raw(
        &self,
        thread_id: &str,
        p: ThreadStartParams,
    ) -> Result<ThreadHandle, RpcError> {
        super::wire::validate_thread_start_security(&p)?;
        let mut params = Map::<String, Value>::new();
        params.insert("threadId".to_owned(), Value::String(thread_id.to_owned()));
        let overrides = thread_overrides_to_wire(&p);
        if !overrides.is_empty() {
            params.insert("overrides".to_owned(), Value::Object(overrides));
        }

        let response = self
            .call_validated(methods::THREAD_RESUME, Value::Object(params))
            .await?;
        let resumed = parse_thread_id(&response).ok_or_else(|| {
            RpcError::InvalidRequest(format!(
                "thread/resume missing thread id in result: {response}"
            ))
        })?;
        if resumed != thread_id {
            return Err(RpcError::InvalidRequest(format!(
                "thread/resume returned mismatched thread id: requested={thread_id} actual={resumed}"
            )));
        }
        Ok(ThreadHandle {
            thread_id: resumed,
            runtime: self.clone(),
        })
    }

    pub async fn thread_fork(&self, thread_id: &str) -> Result<ThreadHandle, RpcError> {
        let mut params = Map::<String, Value>::new();
        params.insert("threadId".to_owned(), Value::String(thread_id.to_owned()));
        let response = self
            .call_validated(methods::THREAD_FORK, Value::Object(params))
            .await?;
        let forked = parse_thread_id(&response).ok_or_else(|| {
            RpcError::InvalidRequest(format!(
                "thread/fork missing thread id in result: {response}"
            ))
        })?;
        Ok(ThreadHandle {
            thread_id: forked,
            runtime: self.clone(),
        })
    }

    /// Archive a thread (logical close on server side).
    /// Allocation: one JSON object with thread id.
    /// Complexity: O(1).
    pub async fn thread_archive(&self, thread_id: &str) -> Result<(), RpcError> {
        let mut params = Map::<String, Value>::new();
        params.insert("threadId".to_owned(), Value::String(thread_id.to_owned()));
        let _ = self
            .call_validated(methods::THREAD_ARCHIVE, Value::Object(params))
            .await?;
        Ok(())
    }

    /// Read one thread by id.
    /// Allocation: serialized params + decoded response object.
    /// Complexity: O(n), n = thread payload size.
    pub async fn thread_read(&self, p: ThreadReadParams) -> Result<ThreadReadResponse, RpcError> {
        let params = serialize_params(methods::THREAD_READ, &p)?;
        let response = self.call_validated(methods::THREAD_READ, params).await?;
        deserialize_result(methods::THREAD_READ, response)
    }

    /// List persisted threads with optional filters and pagination.
    /// Allocation: serialized params + decoded list payload.
    /// Complexity: O(n), n = number of returned threads.
    pub async fn thread_list(&self, p: ThreadListParams) -> Result<ThreadListResponse, RpcError> {
        let params = serialize_params(methods::THREAD_LIST, &p)?;
        let response = self.call_validated(methods::THREAD_LIST, params).await?;
        deserialize_result(methods::THREAD_LIST, response)
    }

    /// List currently loaded thread ids from in-memory sessions.
    /// Allocation: serialized params + decoded list payload.
    /// Complexity: O(n), n = number of returned ids.
    pub async fn thread_loaded_list(
        &self,
        p: ThreadLoadedListParams,
    ) -> Result<ThreadLoadedListResponse, RpcError> {
        let params = serialize_params(methods::THREAD_LOADED_LIST, &p)?;
        let response = self
            .call_validated(methods::THREAD_LOADED_LIST, params)
            .await?;
        deserialize_result(methods::THREAD_LOADED_LIST, response)
    }

    /// List skills for one or more working directories.
    /// Allocation: serialized params + decoded inventory payload.
    /// Complexity: O(n), n = number of returned cwd entries + skill metadata size.
    pub async fn skills_list(&self, p: SkillsListParams) -> Result<SkillsListResponse, RpcError> {
        let params = serialize_params(methods::SKILLS_LIST, &p)?;
        let response = self.call_validated(methods::SKILLS_LIST, params).await?;
        deserialize_result(methods::SKILLS_LIST, response)
    }

    /// Roll back the last `num_turns` turns from a thread.
    /// Allocation: serialized params + decoded response payload.
    /// Complexity: O(n), n = rolled thread payload size.
    pub async fn thread_rollback(
        &self,
        p: ThreadRollbackParams,
    ) -> Result<ThreadRollbackResponse, RpcError> {
        let params = serialize_params(methods::THREAD_ROLLBACK, &p)?;
        let response = self
            .call_validated(methods::THREAD_ROLLBACK, params)
            .await?;
        deserialize_result(methods::THREAD_ROLLBACK, response)
    }

    /// Interrupt one in-flight turn for a thread.
    /// Allocation: one JSON object with thread + turn id.
    /// Complexity: O(1).
    pub async fn turn_interrupt(&self, thread_id: &str, turn_id: &str) -> Result<(), RpcError> {
        let _ = self
            .call_validated(
                methods::TURN_INTERRUPT,
                interrupt_params(thread_id, turn_id),
            )
            .await?;
        Ok(())
    }

    /// Interrupt one in-flight turn with explicit RPC timeout.
    /// Allocation: one JSON object with thread + turn id.
    /// Complexity: O(1).
    pub async fn turn_interrupt_with_timeout(
        &self,
        thread_id: &str,
        turn_id: &str,
        timeout_duration: Duration,
    ) -> Result<(), RpcError> {
        let _ = self
            .call_validated_with_mode_and_timeout(
                methods::TURN_INTERRUPT,
                interrupt_params(thread_id, turn_id),
                RpcValidationMode::KnownMethods,
                timeout_duration,
            )
            .await?;
        Ok(())
    }
}

fn interrupt_params(thread_id: &str, turn_id: &str) -> Value {
    let mut params = Map::<String, Value>::new();
    params.insert("threadId".to_owned(), Value::String(thread_id.to_owned()));
    params.insert("turnId".to_owned(), Value::String(turn_id.to_owned()));
    Value::Object(params)
}

fn ensure_turn_input_not_empty(input: &[InputItem]) -> Result<(), RpcError> {
    if input.is_empty() {
        return Err(RpcError::InvalidRequest(
            "turn input must not be empty".to_owned(),
        ));
    }
    Ok(())
}
