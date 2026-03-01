use std::time::Duration;

use coclai_plugin_core::HookPhase;
use serde_json::{Map, Value};

use crate::errors::RpcError;
use crate::hooks::RuntimeHookConfig;
use crate::runtime::Runtime;
use crate::turn_output::parse_thread_id;

use super::flow::{
    apply_pre_hook_actions_to_session, result_status, HookContextInput, HookExecutionState,
    SessionMutationState,
};
use super::ops::{deserialize_result, serialize_params};
use super::wire::thread_overrides_to_wire;
use super::*;

impl Runtime {
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

        let mut hook_state = HookExecutionState::new(self.next_hook_correlation_id());
        let mut session_state =
            SessionMutationState::from_thread_start(&p, hook_state.metadata.clone());
        let decisions = self
            .execute_pre_hook_phase(
                &mut hook_state,
                HookPhase::PreSessionStart,
                p.cwd.as_deref(),
                p.model.as_deref(),
                None,
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

        let start_cwd = p.cwd.clone();
        let start_model = p.model.clone();
        let result = self.thread_start_raw(p).await;
        let post_thread_id = result.as_ref().ok().map(|thread| thread.thread_id.as_str());
        self.execute_post_hook_phase(
            &mut hook_state,
            HookContextInput {
                phase: HookPhase::PostSessionStart,
                cwd: start_cwd.as_deref(),
                model: start_model.as_deref(),
                thread_id: post_thread_id,
                turn_id: None,
                main_status: Some(result_status(&result)),
            },
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

        let mut hook_state = HookExecutionState::new(self.next_hook_correlation_id());
        let mut session_state =
            SessionMutationState::from_thread_start(&p, hook_state.metadata.clone());
        let decisions = self
            .execute_pre_hook_phase(
                &mut hook_state,
                HookPhase::PreSessionStart,
                p.cwd.as_deref(),
                p.model.as_deref(),
                Some(thread_id),
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

        let resume_cwd = p.cwd.clone();
        let resume_model = p.model.clone();
        let result = self.thread_resume_raw(thread_id, p).await;
        let post_thread_id = result
            .as_ref()
            .ok()
            .map(|thread| thread.thread_id.as_str())
            .or(Some(thread_id));
        self.execute_post_hook_phase(
            &mut hook_state,
            HookContextInput {
                phase: HookPhase::PostSessionStart,
                cwd: resume_cwd.as_deref(),
                model: resume_model.as_deref(),
                thread_id: post_thread_id,
                turn_id: None,
                main_status: Some(result_status(&result)),
            },
            scoped_hooks,
        )
        .await;
        self.publish_hook_report(hook_state.report);
        result
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
            .call_raw("thread/resume", Value::Object(params))
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
        let response = self.call_raw("thread/fork", Value::Object(params)).await?;
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
            .call_raw("thread/archive", Value::Object(params))
            .await?;
        Ok(())
    }

    /// Read one thread by id.
    /// Allocation: serialized params + decoded response object.
    /// Complexity: O(n), n = thread payload size.
    pub async fn thread_read(&self, p: ThreadReadParams) -> Result<ThreadReadResponse, RpcError> {
        let params = serialize_params("thread/read", &p)?;
        let response = self.call_raw("thread/read", params).await?;
        deserialize_result("thread/read", response)
    }

    /// List persisted threads with optional filters and pagination.
    /// Allocation: serialized params + decoded list payload.
    /// Complexity: O(n), n = number of returned threads.
    pub async fn thread_list(&self, p: ThreadListParams) -> Result<ThreadListResponse, RpcError> {
        let params = serialize_params("thread/list", &p)?;
        let response = self.call_raw("thread/list", params).await?;
        deserialize_result("thread/list", response)
    }

    /// List currently loaded thread ids from in-memory sessions.
    /// Allocation: serialized params + decoded list payload.
    /// Complexity: O(n), n = number of returned ids.
    pub async fn thread_loaded_list(
        &self,
        p: ThreadLoadedListParams,
    ) -> Result<ThreadLoadedListResponse, RpcError> {
        let params = serialize_params("thread/loaded/list", &p)?;
        let response = self.call_raw("thread/loaded/list", params).await?;
        deserialize_result("thread/loaded/list", response)
    }

    /// Roll back the last `num_turns` turns from a thread.
    /// Allocation: serialized params + decoded response payload.
    /// Complexity: O(n), n = rolled thread payload size.
    pub async fn thread_rollback(
        &self,
        p: ThreadRollbackParams,
    ) -> Result<ThreadRollbackResponse, RpcError> {
        let params = serialize_params("thread/rollback", &p)?;
        let response = self.call_raw("thread/rollback", params).await?;
        deserialize_result("thread/rollback", response)
    }

    /// Interrupt one in-flight turn for a thread.
    /// Allocation: one JSON object with thread + turn id.
    /// Complexity: O(1).
    pub async fn turn_interrupt(&self, thread_id: &str, turn_id: &str) -> Result<(), RpcError> {
        let mut params = Map::<String, Value>::new();
        params.insert("threadId".to_owned(), Value::String(thread_id.to_owned()));
        params.insert("turnId".to_owned(), Value::String(turn_id.to_owned()));
        let _ = self
            .call_raw("turn/interrupt", Value::Object(params))
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
        let mut params = Map::<String, Value>::new();
        params.insert("threadId".to_owned(), Value::String(thread_id.to_owned()));
        params.insert("turnId".to_owned(), Value::String(turn_id.to_owned()));
        let _ = self
            .call_raw_with_timeout("turn/interrupt", Value::Object(params), timeout_duration)
            .await?;
        Ok(())
    }
}
