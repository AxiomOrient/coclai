use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use coclai_runtime::approvals::ServerRequest;
use coclai_runtime::events::Envelope;
use coclai_runtime::runtime::Runtime;
use coclai_runtime::PluginContractVersion;
use tokio::sync::{broadcast, RwLock};

mod adapter;
mod approval_service;
mod routing;
mod session_service;
mod state;
mod subscription_service;
mod turn_service;
mod wire;

pub use adapter::{RuntimeWebAdapter, WebAdapterFuture, WebPluginAdapter, WebRuntimeStreams};

mod types;

pub use types::*;

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
        session_service::create_session(&self.adapter, &self.state, self.config, tenant_id, request)
            .await
    }

    pub async fn create_turn(
        &self,
        tenant_id: &str,
        session_id: &str,
        request: CreateTurnRequest,
    ) -> Result<CreateTurnResponse, WebError> {
        turn_service::create_turn(&self.adapter, &self.state, tenant_id, session_id, request).await
    }

    pub async fn close_session(
        &self,
        tenant_id: &str,
        session_id: &str,
    ) -> Result<CloseSessionResponse, WebError> {
        session_service::close_session(&self.adapter, &self.state, tenant_id, session_id).await
    }

    pub async fn subscribe_session_events(
        &self,
        tenant_id: &str,
        session_id: &str,
    ) -> Result<broadcast::Receiver<Envelope>, WebError> {
        subscription_service::subscribe_session_events(&self.state, tenant_id, session_id).await
    }

    pub async fn subscribe_session_approvals(
        &self,
        tenant_id: &str,
        session_id: &str,
    ) -> Result<broadcast::Receiver<ServerRequest>, WebError> {
        subscription_service::subscribe_session_approvals(&self.state, tenant_id, session_id).await
    }

    pub async fn post_approval(
        &self,
        tenant_id: &str,
        session_id: &str,
        approval_id: &str,
        payload: ApprovalResponsePayload,
    ) -> Result<(), WebError> {
        approval_service::post_approval(
            &self.adapter,
            &self.state,
            tenant_id,
            session_id,
            approval_id,
            payload,
        )
        .await
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
