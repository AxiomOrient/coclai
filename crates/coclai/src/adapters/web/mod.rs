use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::runtime::approvals::ServerRequest;
use crate::runtime::core::Runtime;
use crate::runtime::events::Envelope;
use tokio::sync::{broadcast, RwLock};

mod adapter;
mod handlers;
mod service;
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
        let streams = service::prepare_spawn(&adapter, &config).await?;
        let state = Arc::new(RwLock::new(state::WebState::default()));
        let handles =
            service::spawn_routing_tasks(Arc::clone(&adapter), Arc::clone(&state), streams);
        let background_tasks = Arc::new(BackgroundTasks::new(handles));

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

    #[cfg(test)]
    pub(crate) async fn debug_server_request_route_miss_counts(&self) -> (u64, u64, u64) {
        let guard = self.state.read().await;
        let metrics = guard.server_request_route_miss;
        (
            metrics.missing_thread_id,
            metrics.missing_session_mapping,
            metrics.missing_approval_topic,
        )
    }

    #[cfg(test)]
    pub(crate) async fn debug_remove_approval_topic(&self, session_id: &str) {
        self.state.write().await.approval_topics.remove(session_id);
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
        // Shared background tasks must stay alive while any clone is still in use.
        if Arc::strong_count(&self.background_tasks) == 1 {
            self.background_tasks.abort_all();
        }
    }
}

#[cfg(test)]
mod tests;
