use std::collections::HashMap;
use std::sync::Arc;

use coclai_runtime::approvals::ServerRequest;
use coclai_runtime::events::Envelope;
use tokio::sync::{broadcast, RwLock};
use uuid::Uuid;

use super::{CreateSessionResponse, WebAdapterConfig, WebError};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SessionRecord {
    pub(super) session_id: String,
    pub(super) tenant_id: String,
    pub(super) artifact_id: String,
    pub(super) thread_id: String,
}

#[derive(Default)]
pub(super) struct WebState {
    pub(super) sessions: HashMap<String, SessionRecord>,
    pub(super) thread_to_session: HashMap<String, String>,
    pub(super) event_topics: HashMap<String, broadcast::Sender<Envelope>>,
    pub(super) approval_topics: HashMap<String, broadcast::Sender<ServerRequest>>,
    pub(super) approval_to_session: HashMap<String, String>,
}

pub(super) async fn assert_thread_access(
    state: &Arc<RwLock<WebState>>,
    tenant_id: &str,
    thread_id: &str,
) -> Result<(), WebError> {
    let state = state.read().await;
    let Some(existing_session_id) = state.thread_to_session.get(thread_id) else {
        return Err(WebError::Forbidden);
    };
    let existing = state
        .sessions
        .get(existing_session_id)
        .ok_or_else(|| WebError::Internal("thread index points to missing session".to_owned()))?;
    if existing.tenant_id != tenant_id {
        return Err(WebError::Forbidden);
    }
    Ok(())
}

pub(super) async fn register_session(
    state: &Arc<RwLock<WebState>>,
    config: WebAdapterConfig,
    tenant_id: &str,
    artifact_id: &str,
    thread_id: &str,
) -> Result<CreateSessionResponse, WebError> {
    let mut state = state.write().await;
    if let Some(existing_session_id) = state.thread_to_session.get(thread_id).cloned() {
        let existing = state
            .sessions
            .get(&existing_session_id)
            .ok_or_else(|| WebError::Internal("thread index points to missing session".to_owned()))?
            .clone();
        if existing.tenant_id != tenant_id {
            return Err(WebError::Forbidden);
        }
        return Ok(CreateSessionResponse {
            session_id: existing.session_id,
            thread_id: existing.thread_id,
        });
    }

    let session_id = new_session_id();
    let session = SessionRecord {
        session_id: session_id.clone(),
        tenant_id: tenant_id.to_owned(),
        artifact_id: artifact_id.to_owned(),
        thread_id: thread_id.to_owned(),
    };

    let (event_tx, _) = broadcast::channel(config.session_event_channel_capacity);
    let (approval_tx, _) = broadcast::channel(config.session_approval_channel_capacity);
    state.sessions.insert(session_id.clone(), session);
    state
        .thread_to_session
        .insert(thread_id.to_owned(), session_id.clone());
    state.event_topics.insert(session_id.clone(), event_tx);
    state
        .approval_topics
        .insert(session_id.clone(), approval_tx);

    Ok(CreateSessionResponse {
        session_id,
        thread_id: thread_id.to_owned(),
    })
}

pub(super) async fn load_owned_session(
    state: &Arc<RwLock<WebState>>,
    tenant_id: &str,
    session_id: &str,
) -> Result<SessionRecord, WebError> {
    let state = state.read().await;
    let session = state
        .sessions
        .get(session_id)
        .ok_or(WebError::InvalidSession)?
        .clone();
    if session.tenant_id != tenant_id {
        return Err(WebError::Forbidden);
    }
    Ok(session)
}

pub(super) async fn close_owned_session(
    state: &Arc<RwLock<WebState>>,
    tenant_id: &str,
    session_id: &str,
) -> Result<SessionRecord, WebError> {
    let mut state = state.write().await;
    let session = state
        .sessions
        .get(session_id)
        .ok_or(WebError::InvalidSession)?
        .clone();
    if session.tenant_id != tenant_id {
        return Err(WebError::Forbidden);
    }

    state.sessions.remove(session_id);
    state.thread_to_session.remove(&session.thread_id);
    state.event_topics.remove(session_id);
    state.approval_topics.remove(session_id);
    state
        .approval_to_session
        .retain(|_, owner_session_id| owner_session_id != session_id);

    Ok(session)
}

pub(super) fn new_session_id() -> String {
    format!("sess_{}", Uuid::new_v4())
}
