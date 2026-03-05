use std::collections::HashSet;
use std::sync::Arc;

use tokio::sync::{broadcast, RwLock};

use crate::runtime::api::ThreadStartParams;
use crate::runtime::approvals::ServerRequest;
use crate::runtime::events::Envelope;

use super::state::{self, WebState};
use super::{
    wire, ApprovalResponsePayload, CloseSessionResponse, CreateSessionRequest,
    CreateSessionResponse, CreateTurnRequest, CreateTurnResponse, WebAdapterConfig, WebError,
    WebPluginAdapter,
};

// --- routing ---

/// Route one live envelope to the owning session topic.
/// Allocation: none. Complexity: O(1).
pub(super) async fn route_session_event(state: &Arc<RwLock<WebState>>, envelope: Envelope) {
    let Some(thread_id) = envelope.thread_id.as_deref() else {
        return;
    };

    let sender = {
        let guard = state.read().await;
        let Some(session_id) = guard.thread_to_session.get(thread_id) else {
            return;
        };
        guard.event_topics.get(session_id).cloned()
    };
    if let Some(sender) = sender {
        let _ = sender.send(envelope);
    }
}

/// Route one server request to the owning session approval topic and index approval ownership.
/// Allocation: one thread id string clone. Complexity: O(1).
pub(super) async fn route_server_request(state: &Arc<RwLock<WebState>>, request: ServerRequest) {
    let Some(thread_id) = extract_thread_id_from_request(&request) else {
        return;
    };

    let sender = {
        let mut guard = state.write().await;
        let Some(session_id) = guard.thread_to_session.get(&thread_id).cloned() else {
            return;
        };
        guard
            .approval_to_session
            .insert(request.approval_id.clone(), session_id.clone());
        guard.approval_topics.get(&session_id).cloned()
    };
    if let Some(sender) = sender {
        let _ = sender.send(request);
    }
}

/// Remove stale approval->session links by reconciling with adapter pending approval set.
/// Allocation: O(n) approval id set snapshot. Complexity: O(n), n = pending approval count.
pub(super) async fn prune_stale_approval_index(
    state: &Arc<RwLock<WebState>>,
    adapter: &Arc<dyn WebPluginAdapter>,
) {
    let active: HashSet<String> = adapter.pending_approval_ids().into_iter().collect();

    let mut guard = state.write().await;
    guard
        .approval_to_session
        .retain(|approval_id, _| active.contains(approval_id));
}

fn extract_thread_id_from_request(request: &ServerRequest) -> Option<String> {
    wire::extract_thread_id_from_server_request_params(&request.params)
}

// --- session_service ---

pub(super) async fn create_session(
    adapter: &Arc<dyn WebPluginAdapter>,
    state: &Arc<RwLock<WebState>>,
    config: WebAdapterConfig,
    tenant_id: &str,
    request: CreateSessionRequest,
) -> Result<CreateSessionResponse, WebError> {
    if request.artifact_id.trim().is_empty() {
        return Err(WebError::InvalidSession);
    }

    if let Some(thread_id) = request.thread_id.as_deref() {
        state::assert_thread_access(state, tenant_id, thread_id).await?;
    }

    let thread_params = ThreadStartParams {
        model: request.model.clone(),
        ..ThreadStartParams::default()
    };
    let thread_id =
        resolve_session_thread_id(adapter, request.thread_id.as_deref(), thread_params).await?;

    state::register_session(state, config, tenant_id, &request.artifact_id, &thread_id).await
}

async fn resolve_session_thread_id(
    adapter: &Arc<dyn WebPluginAdapter>,
    resume_thread_id: Option<&str>,
    thread_params: ThreadStartParams,
) -> Result<String, WebError> {
    match resume_thread_id {
        Some(thread_id) => {
            let resumed_thread_id = adapter.thread_resume(thread_id, thread_params).await?;
            if resumed_thread_id != thread_id {
                return Err(WebError::Internal(format!(
                    "thread/resume returned mismatched thread id: requested={thread_id} actual={resumed_thread_id}"
                )));
            }
            Ok(resumed_thread_id)
        }
        None => adapter.thread_start(thread_params).await,
    }
}

pub(super) async fn close_session(
    adapter: &Arc<dyn WebPluginAdapter>,
    state: &Arc<RwLock<WebState>>,
    tenant_id: &str,
    session_id: &str,
) -> Result<CloseSessionResponse, WebError> {
    let session = state::begin_close_owned_session(state, tenant_id, session_id).await?;
    match adapter.thread_archive(&session.thread_id).await {
        Ok(()) => {
            let closed = state::finalize_close_owned_session(state, tenant_id, session_id).await?;
            Ok(CloseSessionResponse {
                thread_id: closed.thread_id,
                archived: true,
            })
        }
        Err(err) => {
            let rollback = state::rollback_close_owned_session(state, tenant_id, session_id).await;
            if let Err(rollback_err) = rollback {
                return Err(WebError::Internal(format!(
                    "thread/archive failed for session {session_id}: {err}; rollback failed: {rollback_err}"
                )));
            }
            Err(WebError::Internal(format!(
                "thread/archive failed for session {session_id}: {err}"
            )))
        }
    }
}

// --- turn_service ---

pub(super) async fn create_turn(
    adapter: &Arc<dyn WebPluginAdapter>,
    state: &Arc<RwLock<WebState>>,
    tenant_id: &str,
    session_id: &str,
    request: CreateTurnRequest,
) -> Result<CreateTurnResponse, WebError> {
    let session = state::load_owned_session(state, tenant_id, session_id).await?;
    let params = wire::normalize_turn_start_params(&session.thread_id, request.task)?;
    let result = adapter.turn_start(params).await?;
    let turn_id = wire::parse_turn_id_from_turn_result(&result).ok_or_else(|| {
        WebError::Internal(format!("turn/start missing turn id in result: {result}"))
    })?;
    Ok(CreateTurnResponse { turn_id })
}

// --- subscription_service ---

pub(super) async fn subscribe_session_events(
    state: &Arc<RwLock<WebState>>,
    tenant_id: &str,
    session_id: &str,
) -> Result<broadcast::Receiver<Envelope>, WebError> {
    subscribe_session_topic(state, tenant_id, session_id, |state, id| {
        state.event_topics.get(id).cloned()
    })
    .await
}

pub(super) async fn subscribe_session_approvals(
    state: &Arc<RwLock<WebState>>,
    tenant_id: &str,
    session_id: &str,
) -> Result<broadcast::Receiver<ServerRequest>, WebError> {
    subscribe_session_topic(state, tenant_id, session_id, |state, id| {
        state.approval_topics.get(id).cloned()
    })
    .await
}

async fn subscribe_session_topic<T: Clone>(
    state: &Arc<RwLock<WebState>>,
    tenant_id: &str,
    session_id: &str,
    topic_lookup: impl Fn(&WebState, &str) -> Option<broadcast::Sender<T>>,
) -> Result<broadcast::Receiver<T>, WebError> {
    let _ = state::load_owned_session(state, tenant_id, session_id).await?;
    let sender = {
        let state = state.read().await;
        topic_lookup(&state, session_id).ok_or(WebError::InvalidSession)?
    };
    Ok(sender.subscribe())
}

// --- approval_service ---

pub(super) async fn post_approval(
    adapter: &Arc<dyn WebPluginAdapter>,
    state: &Arc<RwLock<WebState>>,
    tenant_id: &str,
    session_id: &str,
    approval_id: &str,
    payload: ApprovalResponsePayload,
) -> Result<(), WebError> {
    let _ = state::load_owned_session(state, tenant_id, session_id).await?;

    let owner = {
        let state = state.read().await;
        state.approval_to_session.get(approval_id).cloned()
    };
    let Some(owner_session_id) = owner else {
        return Err(WebError::InvalidApproval);
    };
    if owner_session_id != session_id {
        return Err(WebError::Forbidden);
    }

    let result = payload.into_result_payload()?;
    adapter.respond_approval_ok(approval_id, result).await?;
    state.write().await.approval_to_session.remove(approval_id);
    Ok(())
}
