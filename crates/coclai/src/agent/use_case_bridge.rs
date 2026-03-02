use std::future::Future;

use serde_json::{json, Map, Value};

use crate::agent::{
    AgentDispatchError, AgentState, CapabilityInvocation, CapabilityResponse, CoclaiAgent,
    ManagedAppServer,
};
use crate::application::appserver as appserver_uc;
use crate::application::capability_dispatch::{decode_workflow_config, encode_workflow_config};
use crate::application::quick_run as quick_run_uc;
use crate::application::workflow as workflow_uc;
use crate::appserver::{methods, AppServer};
use crate::ergonomic::WorkflowConfig;

use super::payload_parse::{
    maybe_profile_field, optional_string_field, payload_as_object, require_string_field,
};

impl CoclaiAgent {
    pub(super) fn acquire_state_lock(
        &self,
        capability_id: &str,
    ) -> Result<std::sync::MutexGuard<'_, AgentState>, AgentDispatchError> {
        self.state
            .lock()
            .map_err(|err| AgentDispatchError::BackendFailure {
                capability_id: capability_id.to_owned(),
                message: format!("state lock poisoned: {err}"),
            })
    }

    pub(super) fn execute_async_use_case<T, E, F>(
        &self,
        capability_id: &str,
        fut: F,
    ) -> Result<T, AgentDispatchError>
    where
        E: std::fmt::Display,
        F: Future<Output = Result<T, E>>,
    {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|err| AgentDispatchError::BackendFailure {
                capability_id: capability_id.to_owned(),
                message: format!("failed to build tokio runtime: {err}"),
            })?;
        runtime
            .block_on(fut)
            .map_err(|err| AgentDispatchError::BackendFailure {
                capability_id: capability_id.to_owned(),
                message: err.to_string(),
            })
    }

    pub(super) fn resolve_or_connect_appserver(
        &self,
        capability_id: &str,
        payload: &Map<String, Value>,
    ) -> Result<(String, AppServer), AgentDispatchError> {
        let connection_id =
            optional_string_field(payload, "connection_id").unwrap_or_else(|| "default".to_owned());

        if let Some(existing) = self
            .acquire_state_lock(capability_id)?
            .appservers
            .get(&connection_id)
        {
            return Ok((connection_id, existing.appserver.clone()));
        }

        let connected = self.execute_async_use_case(capability_id, AppServer::connect_default())?;
        self.acquire_state_lock(capability_id)?.appservers.insert(
            connection_id.clone(),
            ManagedAppServer {
                appserver: connected.clone(),
                server_requests: None,
            },
        );
        self.state_store
            .upsert_connection_state(&connection_id, json!({"connected": true}))
            .map_err(|message| AgentDispatchError::BackendFailure {
                capability_id: capability_id.to_owned(),
                message: format!("failed to persist connection state: {message}"),
            })?;

        Ok((connection_id, connected))
    }

    pub(super) fn allocate_workflow_id(
        &self,
        capability_id: &str,
    ) -> Result<String, AgentDispatchError> {
        let mut state = self.acquire_state_lock(capability_id)?;
        state.next_workflow_id = state.next_workflow_id.saturating_add(1);
        Ok(format!("wf-{}", state.next_workflow_id))
    }

    pub(super) fn persist_workflow_config(
        &self,
        capability_id: &str,
        workflow_id: &str,
        config: &WorkflowConfig,
    ) -> Result<(), AgentDispatchError> {
        self.state_store
            .upsert_workflow_config(workflow_id, encode_workflow_config(config))
            .map_err(|message| AgentDispatchError::BackendFailure {
                capability_id: capability_id.to_owned(),
                message: format!("failed to persist workflow `{workflow_id}`: {message}"),
            })
    }

    pub(super) fn load_workflow_config(
        &self,
        capability_id: &str,
        workflow_id: &str,
    ) -> Result<WorkflowConfig, AgentDispatchError> {
        let Some(raw) = self
            .state_store
            .load_workflow_config(workflow_id)
            .map_err(|message| AgentDispatchError::BackendFailure {
                capability_id: capability_id.to_owned(),
                message: format!("failed to load workflow `{workflow_id}`: {message}"),
            })?
        else {
            return Err(AgentDispatchError::InvalidPayload {
                capability_id: capability_id.to_owned(),
                message: format!("unknown workflow_id: {workflow_id}"),
            });
        };

        decode_workflow_config(&raw).map_err(|message| AgentDispatchError::BackendFailure {
            capability_id: capability_id.to_owned(),
            message: format!("workflow store row for `{workflow_id}` is invalid: {message}"),
        })
    }

    pub(super) fn is_forwarded_rpc_capability(capability_id: &str) -> bool {
        matches!(
            capability_id,
            methods::THREAD_START
                | methods::THREAD_RESUME
                | methods::THREAD_FORK
                | methods::THREAD_ARCHIVE
                | methods::THREAD_READ
                | methods::THREAD_LIST
                | methods::THREAD_LOADED_LIST
                | methods::THREAD_ROLLBACK
                | methods::TURN_START
                | methods::TURN_INTERRUPT
        )
    }

    pub(super) fn dispatch_exposed_capability(
        &self,
        invocation: CapabilityInvocation,
        descriptor_capability_id: &str,
    ) -> Result<CapabilityResponse, AgentDispatchError> {
        match invocation.capability_id.as_str() {
            "quick_run" => return self.handle_quick_run(invocation),
            "quick_run_with_profile" => return self.handle_quick_run_with_profile(invocation),
            "workflow/connect" => return self.handle_workflow_connect(invocation),
            "workflow/run" => return self.handle_workflow_run(invocation),
            "workflow/session/setup" => return self.handle_workflow_session_setup(invocation),
            "appserver/request/json" | "appserver/request/typed" => {
                return self.handle_appserver_request_json(invocation);
            }
            "appserver/notify/json" => return self.handle_appserver_notify_json(invocation),
            "appserver/server-requests/take" => {
                return self.handle_appserver_server_requests_take(invocation);
            }
            "appserver/server-requests/respond/ok" => {
                return self.handle_appserver_server_requests_respond_ok(invocation);
            }
            "appserver/server-requests/respond/err" => {
                return self.handle_appserver_server_requests_respond_err(invocation);
            }
            _ => {}
        }

        if Self::is_forwarded_rpc_capability(descriptor_capability_id) {
            return self.handle_rpc_forward(invocation);
        }

        Err(AgentDispatchError::BackendFailure {
            capability_id: descriptor_capability_id.to_owned(),
            message: "capability is exposed but no dispatch branch exists".to_owned(),
        })
    }

    pub(super) fn handle_quick_run(
        &self,
        invocation: CapabilityInvocation,
    ) -> Result<CapabilityResponse, AgentDispatchError> {
        let obj = payload_as_object(&invocation)?;
        let cwd = require_string_field(&invocation.capability_id, obj, "cwd")?;
        let prompt = require_string_field(&invocation.capability_id, obj, "prompt")?;
        let output = quick_run_uc::execute_quick_run(self.codex_gateway.as_ref(), &cwd, &prompt)
            .map_err(|message| AgentDispatchError::BackendFailure {
                capability_id: invocation.capability_id.clone(),
                message,
            })?;
        Ok(CapabilityResponse {
            capability_id: invocation.capability_id,
            correlation_id: invocation.correlation_id,
            result: output,
        })
    }

    pub(super) fn handle_quick_run_with_profile(
        &self,
        invocation: CapabilityInvocation,
    ) -> Result<CapabilityResponse, AgentDispatchError> {
        let obj = payload_as_object(&invocation)?;
        let cwd = require_string_field(&invocation.capability_id, obj, "cwd")?;
        let prompt = require_string_field(&invocation.capability_id, obj, "prompt")?;
        let profile =
            maybe_profile_field(&invocation.capability_id, obj, "profile")?.ok_or_else(|| {
                AgentDispatchError::InvalidPayload {
                    capability_id: invocation.capability_id.clone(),
                    message: "payload.profile is required".to_owned(),
                }
            })?;

        let output = quick_run_uc::execute_quick_run_with_profile(
            self.codex_gateway.as_ref(),
            &cwd,
            &prompt,
            profile,
        )
        .map_err(|message| AgentDispatchError::BackendFailure {
            capability_id: invocation.capability_id.clone(),
            message,
        })?;
        Ok(CapabilityResponse {
            capability_id: invocation.capability_id,
            correlation_id: invocation.correlation_id,
            result: output,
        })
    }

    pub(super) fn handle_workflow_connect(
        &self,
        invocation: CapabilityInvocation,
    ) -> Result<CapabilityResponse, AgentDispatchError> {
        let obj = payload_as_object(&invocation)?;
        let cwd = require_string_field(&invocation.capability_id, obj, "cwd")?;

        let workflow_id = if let Some(value) = optional_string_field(obj, "workflow_id") {
            value
        } else {
            self.allocate_workflow_id(&invocation.capability_id)?
        };

        let profile = maybe_profile_field(&invocation.capability_id, obj, "profile")?;
        let config = workflow_uc::make_workflow_config(cwd.clone(), profile);
        self.persist_workflow_config(&invocation.capability_id, &workflow_id, &config)?;

        Ok(CapabilityResponse {
            capability_id: invocation.capability_id,
            correlation_id: invocation.correlation_id,
            result: workflow_uc::render_connect_result(&workflow_id, &cwd),
        })
    }

    pub(super) fn handle_workflow_run(
        &self,
        invocation: CapabilityInvocation,
    ) -> Result<CapabilityResponse, AgentDispatchError> {
        let obj = payload_as_object(&invocation)?;
        let workflow_id = require_string_field(&invocation.capability_id, obj, "workflow_id")?;
        let prompt = require_string_field(&invocation.capability_id, obj, "prompt")?;
        let config = self.load_workflow_config(&invocation.capability_id, &workflow_id)?;

        let result = self.execute_async_use_case(
            &invocation.capability_id,
            workflow_uc::execute_workflow_run(config, prompt),
        )?;

        Ok(CapabilityResponse {
            capability_id: invocation.capability_id,
            correlation_id: invocation.correlation_id,
            result: workflow_uc::render_run_result(&workflow_id, result),
        })
    }

    pub(super) fn handle_workflow_session_setup(
        &self,
        invocation: CapabilityInvocation,
    ) -> Result<CapabilityResponse, AgentDispatchError> {
        let obj = payload_as_object(&invocation)?;
        let workflow_id = require_string_field(&invocation.capability_id, obj, "workflow_id")?;
        let config = self.load_workflow_config(&invocation.capability_id, &workflow_id)?;

        let thread_id = self.execute_async_use_case(
            &invocation.capability_id,
            workflow_uc::execute_session_setup(config),
        )?;

        Ok(CapabilityResponse {
            capability_id: invocation.capability_id,
            correlation_id: invocation.correlation_id,
            result: workflow_uc::render_session_setup_result(&workflow_id, &thread_id),
        })
    }

    pub(super) fn handle_appserver_request_json(
        &self,
        invocation: CapabilityInvocation,
    ) -> Result<CapabilityResponse, AgentDispatchError> {
        let obj = payload_as_object(&invocation)?;
        let method = require_string_field(&invocation.capability_id, obj, "method")?;
        let params = obj.get("params").cloned().unwrap_or_else(|| json!({}));
        let (connection_id, appserver) =
            self.resolve_or_connect_appserver(&invocation.capability_id, obj)?;

        let method_for_call = method.clone();
        let response = self.execute_async_use_case(&invocation.capability_id, async move {
            appserver.request_json(&method_for_call, params).await
        })?;

        Ok(CapabilityResponse {
            capability_id: invocation.capability_id,
            correlation_id: invocation.correlation_id,
            result: appserver_uc::render_request_result(&connection_id, &method, response),
        })
    }

    pub(super) fn handle_appserver_notify_json(
        &self,
        invocation: CapabilityInvocation,
    ) -> Result<CapabilityResponse, AgentDispatchError> {
        let obj = payload_as_object(&invocation)?;
        let method = require_string_field(&invocation.capability_id, obj, "method")?;
        let params = obj.get("params").cloned().unwrap_or_else(|| json!({}));
        let (connection_id, appserver) =
            self.resolve_or_connect_appserver(&invocation.capability_id, obj)?;

        let method_for_call = method.clone();
        self.execute_async_use_case(&invocation.capability_id, async move {
            appserver.notify_json(&method_for_call, params).await
        })?;

        Ok(CapabilityResponse {
            capability_id: invocation.capability_id,
            correlation_id: invocation.correlation_id,
            result: appserver_uc::render_notify_result(&connection_id, &method),
        })
    }

    pub(super) fn handle_appserver_server_requests_take(
        &self,
        invocation: CapabilityInvocation,
    ) -> Result<CapabilityResponse, AgentDispatchError> {
        let obj = payload_as_object(&invocation)?;
        let max_items = appserver_uc::resolve_server_request_take_limit(obj);

        let (connection_id, appserver) =
            self.resolve_or_connect_appserver(&invocation.capability_id, obj)?;

        let needs_take = self
            .acquire_state_lock(&invocation.capability_id)?
            .appservers
            .get(&connection_id)
            .map(|entry| entry.server_requests.is_none())
            .unwrap_or(true);

        if needs_take {
            let rx = self.execute_async_use_case(&invocation.capability_id, async move {
                appserver.take_server_requests().await
            })?;
            if let Some(entry) = self
                .acquire_state_lock(&invocation.capability_id)?
                .appservers
                .get_mut(&connection_id)
            {
                if entry.server_requests.is_none() {
                    entry.server_requests = Some(rx);
                }
            }
        }

        let (items, disconnected) = {
            let mut state = self.acquire_state_lock(&invocation.capability_id)?;
            let entry = state.appservers.get_mut(&connection_id).ok_or_else(|| {
                AgentDispatchError::BackendFailure {
                    capability_id: invocation.capability_id.clone(),
                    message: format!("missing appserver connection state: {connection_id}"),
                }
            })?;
            let rx = entry.server_requests.as_mut().ok_or_else(|| {
                AgentDispatchError::BackendFailure {
                    capability_id: invocation.capability_id.clone(),
                    message: "server request receiver not initialized".to_owned(),
                }
            })?;

            let (next_items, was_disconnected) =
                appserver_uc::collect_server_requests(rx, max_items).map_err(|message| {
                    AgentDispatchError::BackendFailure {
                        capability_id: invocation.capability_id.clone(),
                        message,
                    }
                })?;

            if was_disconnected {
                entry.server_requests = None;
            }

            (next_items, was_disconnected)
        };

        Ok(CapabilityResponse {
            capability_id: invocation.capability_id,
            correlation_id: invocation.correlation_id,
            result: appserver_uc::render_server_request_take_result(
                &connection_id,
                items,
                disconnected,
            ),
        })
    }

    pub(super) fn handle_appserver_server_requests_respond_ok(
        &self,
        invocation: CapabilityInvocation,
    ) -> Result<CapabilityResponse, AgentDispatchError> {
        let obj = payload_as_object(&invocation)?;
        let approval_id = require_string_field(&invocation.capability_id, obj, "approval_id")?;
        let result = obj.get("result").cloned().unwrap_or_else(|| json!({}));
        let (connection_id, appserver) =
            self.resolve_or_connect_appserver(&invocation.capability_id, obj)?;

        let approval_id_for_call = approval_id.clone();
        self.execute_async_use_case(&invocation.capability_id, async move {
            appserver
                .respond_server_request_ok(&approval_id_for_call, result)
                .await
        })?;

        Ok(CapabilityResponse {
            capability_id: invocation.capability_id,
            correlation_id: invocation.correlation_id,
            result: appserver_uc::render_server_request_ack(&connection_id, &approval_id),
        })
    }

    pub(super) fn handle_appserver_server_requests_respond_err(
        &self,
        invocation: CapabilityInvocation,
    ) -> Result<CapabilityResponse, AgentDispatchError> {
        let obj = payload_as_object(&invocation)?;
        let approval_id = require_string_field(&invocation.capability_id, obj, "approval_id")?;
        let error_obj = obj.get("error").and_then(Value::as_object).ok_or_else(|| {
            AgentDispatchError::InvalidPayload {
                capability_id: invocation.capability_id.clone(),
                message: "payload.error must be an object with code/message".to_owned(),
            }
        })?;
        let rpc_error = appserver_uc::parse_rpc_error_object(error_obj).map_err(|message| {
            AgentDispatchError::InvalidPayload {
                capability_id: invocation.capability_id.clone(),
                message,
            }
        })?;
        let (connection_id, appserver) =
            self.resolve_or_connect_appserver(&invocation.capability_id, obj)?;

        let approval_id_for_call = approval_id.clone();
        self.execute_async_use_case(&invocation.capability_id, async move {
            appserver
                .respond_server_request_err(&approval_id_for_call, rpc_error)
                .await
        })?;

        Ok(CapabilityResponse {
            capability_id: invocation.capability_id,
            correlation_id: invocation.correlation_id,
            result: appserver_uc::render_server_request_ack(&connection_id, &approval_id),
        })
    }

    pub(super) fn handle_rpc_forward(
        &self,
        invocation: CapabilityInvocation,
    ) -> Result<CapabilityResponse, AgentDispatchError> {
        let obj = payload_as_object(&invocation)?;
        let params = obj.get("params").cloned().unwrap_or_else(|| json!({}));
        let (connection_id, appserver) =
            self.resolve_or_connect_appserver(&invocation.capability_id, obj)?;
        let capability_id = invocation.capability_id.clone();
        let capability_id_for_call = capability_id.clone();

        let response = self.execute_async_use_case(&invocation.capability_id, async move {
            appserver
                .request_json(&capability_id_for_call, params)
                .await
        })?;

        Ok(CapabilityResponse {
            capability_id: invocation.capability_id,
            correlation_id: invocation.correlation_id,
            result: appserver_uc::render_rpc_forward_result(
                &connection_id,
                &capability_id,
                response,
            ),
        })
    }
}
