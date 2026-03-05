use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::plugin::PluginContractVersion;
use crate::runtime::approvals::ServerRequest;
use crate::runtime::core::Runtime;
use crate::runtime::events::Envelope;
use crate::runtime::rpc_contract::methods as events;
use tokio::sync::{broadcast, RwLock};

mod adapter;
mod handlers;
mod state;
mod wire;

#[cfg(test)]
pub(crate) use adapter::WebAdapterFuture;
pub use adapter::{RuntimeWebAdapter, WebPluginAdapter, WebRuntimeStreams};

mod types;

pub use types::{
    ApprovalResponsePayload, CloseSessionResponse, CreateSessionRequest, CreateSessionResponse,
    CreateTurnRequest, CreateTurnResponse, WebAdapterConfig, WebError,
};

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
        validate_web_adapter_config(&config)?;
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
                        handle_live_event(&state_for_events, &adapter_for_events, envelope).await;
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });

        let approvals_task = tokio::spawn(async move {
            while let Some(request) = request_rx.recv().await {
                handlers::prune_stale_approval_index(&state_for_approvals, &adapter_for_approvals)
                    .await;
                handlers::route_server_request(&state_for_approvals, request).await;
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
        handlers::create_session(&self.adapter, &self.state, self.config, tenant_id, request).await
    }

    pub async fn create_turn(
        &self,
        tenant_id: &str,
        session_id: &str,
        request: CreateTurnRequest,
    ) -> Result<CreateTurnResponse, WebError> {
        handlers::create_turn(&self.adapter, &self.state, tenant_id, session_id, request).await
    }

    pub async fn close_session(
        &self,
        tenant_id: &str,
        session_id: &str,
    ) -> Result<CloseSessionResponse, WebError> {
        handlers::close_session(&self.adapter, &self.state, tenant_id, session_id).await
    }

    pub async fn subscribe_session_events(
        &self,
        tenant_id: &str,
        session_id: &str,
    ) -> Result<broadcast::Receiver<Envelope>, WebError> {
        handlers::subscribe_session_events(&self.state, tenant_id, session_id).await
    }

    pub async fn subscribe_session_approvals(
        &self,
        tenant_id: &str,
        session_id: &str,
    ) -> Result<broadcast::Receiver<ServerRequest>, WebError> {
        handlers::subscribe_session_approvals(&self.state, tenant_id, session_id).await
    }

    pub async fn post_approval(
        &self,
        tenant_id: &str,
        session_id: &str,
        approval_id: &str,
        payload: ApprovalResponsePayload,
    ) -> Result<(), WebError> {
        handlers::post_approval(
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
        // `abort_all` is internally idempotent; no extra refcount race checks needed here.
        self.background_tasks.abort_all();
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

/// Validate capacity fields before spawning background tasks.
/// Allocation: none. Complexity: O(1).
fn validate_web_adapter_config(config: &WebAdapterConfig) -> Result<(), WebError> {
    ensure_positive_capacity(
        "session_event_channel_capacity",
        config.session_event_channel_capacity,
    )?;
    ensure_positive_capacity(
        "session_approval_channel_capacity",
        config.session_approval_channel_capacity,
    )?;
    Ok(())
}

fn ensure_positive_capacity(name: &str, value: usize) -> Result<(), WebError> {
    if value > 0 {
        return Ok(());
    }
    Err(WebError::InvalidConfig(format!("{name} must be > 0")))
}

/// Handle one inbound live envelope: route to sessions, then prune stale approval index if needed.
/// Side effects: state write + optional adapter call for pruning. Complexity: O(1) amortised.
async fn handle_live_event(
    state: &Arc<RwLock<state::WebState>>,
    adapter: &Arc<dyn WebPluginAdapter>,
    envelope: crate::runtime::events::Envelope,
) {
    let should_prune = envelope.method.as_deref() == Some(events::APPROVAL_ACK);
    handlers::route_session_event(state, envelope).await;
    if should_prune {
        handlers::prune_stale_approval_index(state, adapter).await;
    }
}

#[cfg(test)]
mod tests;
