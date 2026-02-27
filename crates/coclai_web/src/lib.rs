use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use coclai_runtime::api::ThreadStartParams;
use coclai_runtime::approvals::ServerRequest;
use coclai_runtime::events::Envelope;
use coclai_runtime::runtime::Runtime;
use coclai_runtime::turn_output::parse_turn_id;
use coclai_runtime::PluginContractVersion;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use tokio::sync::{broadcast, RwLock};

mod adapter;
mod routing;
mod state;
mod wire;

pub use adapter::{RuntimeWebAdapter, WebAdapterFuture, WebPluginAdapter, WebRuntimeStreams};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CreateSessionRequest {
    pub artifact_id: String,
    pub model: Option<String>,
    pub thread_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CreateSessionResponse {
    pub session_id: String,
    pub thread_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CloseSessionResponse {
    pub thread_id: String,
    pub archived: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CreateTurnRequest {
    pub task: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CreateTurnResponse {
    pub turn_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalResponsePayload {
    #[serde(default)]
    pub decision: Option<Value>,
    #[serde(default)]
    pub result: Option<Value>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WebAdapterConfig {
    pub session_event_channel_capacity: usize,
    pub session_approval_channel_capacity: usize,
}

impl Default for WebAdapterConfig {
    fn default() -> Self {
        Self {
            session_event_channel_capacity: 512,
            session_approval_channel_capacity: 128,
        }
    }
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum WebError {
    #[error("invalid session")]
    InvalidSession,
    #[error("runtime already bound to a web adapter")]
    AlreadyBound,
    #[error("invalid approval")]
    InvalidApproval,
    #[error("invalid config: {0}")]
    InvalidConfig(String),
    #[error("invalid turn payload")]
    InvalidTurnPayload,
    #[error("invalid approval payload")]
    InvalidApprovalPayload,
    #[error(
        "incompatible plugin contract: expected=v{expected_major}.{expected_minor} actual=v{actual_major}.{actual_minor}"
    )]
    IncompatibleContract {
        expected_major: u16,
        expected_minor: u16,
        actual_major: u16,
        actual_minor: u16,
    },
    #[error("forbidden")]
    Forbidden,
    #[error("internal error: {0}")]
    Internal(String),
}

#[derive(Clone)]
pub struct WebAdapter {
    adapter: Arc<dyn WebPluginAdapter>,
    config: WebAdapterConfig,
    state: Arc<RwLock<state::WebState>>,
    background_tasks: Arc<BackgroundTasks>,
}

#[derive(Debug)]
struct BackgroundTasks {
    aborted: AtomicBool,
    handles: Vec<tokio::task::AbortHandle>,
}

impl BackgroundTasks {
    fn new(handles: Vec<tokio::task::AbortHandle>) -> Self {
        Self {
            aborted: AtomicBool::new(false),
            handles,
        }
    }

    fn abort_all(&self) {
        if self.aborted.swap(true, Ordering::AcqRel) {
            return;
        }
        for handle in &self.handles {
            handle.abort();
        }
    }
}

impl WebAdapter {
    pub async fn spawn(runtime: Runtime, config: WebAdapterConfig) -> Result<Self, WebError> {
        let adapter: Arc<dyn WebPluginAdapter> = Arc::new(RuntimeWebAdapter::new(runtime));
        Self::spawn_with_adapter(adapter, config).await
    }

    pub async fn spawn_with_adapter(
        adapter: Arc<dyn WebPluginAdapter>,
        config: WebAdapterConfig,
    ) -> Result<Self, WebError> {
        if config.session_event_channel_capacity == 0 {
            return Err(WebError::InvalidConfig(
                "session_event_channel_capacity must be > 0".to_owned(),
            ));
        }
        if config.session_approval_channel_capacity == 0 {
            return Err(WebError::InvalidConfig(
                "session_approval_channel_capacity must be > 0".to_owned(),
            ));
        }
        ensure_adapter_contract_compatible(adapter.as_ref())?;

        let WebRuntimeStreams {
            mut request_rx,
            mut live_rx,
        } = adapter.take_streams().await?;
        let adapter_for_events = Arc::clone(&adapter);
        let adapter_for_approvals = Arc::clone(&adapter);

        let state = Arc::new(RwLock::new(state::WebState::default()));
        let state_for_events = Arc::clone(&state);
        let state_for_approvals = Arc::clone(&state);

        let events_task = tokio::spawn(async move {
            loop {
                match live_rx.recv().await {
                    Ok(envelope) => {
                        let should_prune = envelope.method.as_deref() == Some("approval/ack");
                        routing::route_session_event(&state_for_events, envelope).await;
                        if should_prune {
                            routing::prune_stale_approval_index(
                                &state_for_events,
                                &adapter_for_events,
                            )
                            .await;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });

        let approvals_task = tokio::spawn(async move {
            while let Some(request) = request_rx.recv().await {
                routing::prune_stale_approval_index(&state_for_approvals, &adapter_for_approvals)
                    .await;
                routing::route_server_request(&state_for_approvals, request).await;
            }
        });

        let background_tasks = Arc::new(BackgroundTasks::new(vec![
            events_task.abort_handle(),
            approvals_task.abort_handle(),
        ]));

        Ok(Self {
            adapter,
            config,
            state,
            background_tasks,
        })
    }

    pub async fn create_session(
        &self,
        tenant_id: &str,
        request: CreateSessionRequest,
    ) -> Result<CreateSessionResponse, WebError> {
        if request.artifact_id.trim().is_empty() {
            return Err(WebError::InvalidSession);
        }

        if let Some(thread_id) = request.thread_id.as_deref() {
            state::assert_thread_access(&self.state, tenant_id, thread_id).await?;
        }

        let thread_params = ThreadStartParams {
            model: request.model.clone(),
            ..ThreadStartParams::default()
        };
        let thread_id = match request.thread_id.as_deref() {
            Some(thread_id) => {
                let resumed_thread_id =
                    self.adapter.thread_resume(thread_id, thread_params).await?;
                if resumed_thread_id != thread_id {
                    return Err(WebError::Internal(format!(
                        "thread/resume returned mismatched thread id: requested={thread_id} actual={resumed_thread_id}"
                    )));
                }
                resumed_thread_id
            }
            None => self.adapter.thread_start(thread_params).await?,
        };

        state::register_session(
            &self.state,
            self.config,
            tenant_id,
            &request.artifact_id,
            &thread_id,
        )
        .await
    }

    pub async fn create_turn(
        &self,
        tenant_id: &str,
        session_id: &str,
        request: CreateTurnRequest,
    ) -> Result<CreateTurnResponse, WebError> {
        let session = state::load_owned_session(&self.state, tenant_id, session_id).await?;
        let params = wire::normalize_turn_start_params(&session.thread_id, &request.task)?;
        let result = self.adapter.turn_start(params).await?;
        let turn_id = parse_turn_id(&result).ok_or_else(|| {
            WebError::Internal(format!("turn/start missing turn id in result: {result}"))
        })?;
        Ok(CreateTurnResponse { turn_id })
    }

    pub async fn close_session(
        &self,
        tenant_id: &str,
        session_id: &str,
    ) -> Result<CloseSessionResponse, WebError> {
        let session = state::close_owned_session(&self.state, tenant_id, session_id).await?;
        let archived = self
            .adapter
            .thread_archive(&session.thread_id)
            .await
            .is_ok();
        Ok(CloseSessionResponse {
            thread_id: session.thread_id,
            archived,
        })
    }

    pub async fn subscribe_session_events(
        &self,
        tenant_id: &str,
        session_id: &str,
    ) -> Result<broadcast::Receiver<Envelope>, WebError> {
        let _ = state::load_owned_session(&self.state, tenant_id, session_id).await?;
        let sender = {
            let state = self.state.read().await;
            state
                .event_topics
                .get(session_id)
                .cloned()
                .ok_or(WebError::InvalidSession)?
        };
        Ok(sender.subscribe())
    }

    pub async fn subscribe_session_approvals(
        &self,
        tenant_id: &str,
        session_id: &str,
    ) -> Result<broadcast::Receiver<ServerRequest>, WebError> {
        let _ = state::load_owned_session(&self.state, tenant_id, session_id).await?;
        let sender = {
            let state = self.state.read().await;
            state
                .approval_topics
                .get(session_id)
                .cloned()
                .ok_or(WebError::InvalidSession)?
        };
        Ok(sender.subscribe())
    }

    pub async fn post_approval(
        &self,
        tenant_id: &str,
        session_id: &str,
        approval_id: &str,
        payload: ApprovalResponsePayload,
    ) -> Result<(), WebError> {
        let _ = state::load_owned_session(&self.state, tenant_id, session_id).await?;

        let owner = {
            let state = self.state.read().await;
            state.approval_to_session.get(approval_id).cloned()
        };
        let Some(owner_session_id) = owner else {
            return Err(WebError::InvalidApproval);
        };
        if owner_session_id != session_id {
            return Err(WebError::Forbidden);
        }

        let result = payload.into_result_payload()?;
        self.adapter
            .respond_approval_ok(approval_id, result)
            .await?;
        self.state
            .write()
            .await
            .approval_to_session
            .remove(approval_id);
        Ok(())
    }
}

pub fn new_session_id() -> String {
    state::new_session_id()
}

pub fn serialize_sse_envelope(envelope: &Envelope) -> Result<String, WebError> {
    wire::serialize_sse_envelope(envelope)
}

impl Drop for WebAdapter {
    fn drop(&mut self) {
        if Arc::strong_count(&self.background_tasks) == 1 {
            self.background_tasks.abort_all();
        }
    }
}

fn ensure_adapter_contract_compatible(adapter: &dyn WebPluginAdapter) -> Result<(), WebError> {
    let expected = PluginContractVersion::CURRENT;
    let actual = adapter.plugin_contract_version();
    if expected.is_compatible_with(actual) {
        Ok(())
    } else {
        Err(WebError::IncompatibleContract {
            expected_major: expected.major,
            expected_minor: expected.minor,
            actual_major: actual.major,
            actual_minor: actual.minor,
        })
    }
}

#[cfg(test)]
mod tests;
