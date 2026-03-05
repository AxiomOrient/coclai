use std::collections::HashMap;
use std::sync::Arc;

use crate::runtime::approvals::ServerRequest;
use crate::runtime::events::Envelope;
use tokio::sync::{broadcast, RwLock};
use uuid::Uuid;

use super::{CreateSessionResponse, WebAdapterConfig, WebError};

const THREAD_INDEX_INCONSISTENT: &str = "thread index points to missing session";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SessionLifecycle {
    Active,
    Closing,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SessionRecord {
    pub(super) session_id: String,
    pub(super) tenant_id: String,
    pub(super) artifact_id: String,
    pub(super) thread_id: String,
    pub(super) lifecycle: SessionLifecycle,
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
    let Some(existing) = session_from_thread_index(&state, thread_id)? else {
        return Err(WebError::Forbidden);
    };
    ensure_tenant_owns_session(existing, tenant_id)?;
    ensure_session_active(existing)
}

pub(super) async fn register_session(
    state: &Arc<RwLock<WebState>>,
    config: WebAdapterConfig,
    tenant_id: &str,
    artifact_id: &str,
    thread_id: &str,
) -> Result<CreateSessionResponse, WebError> {
    let mut state = state.write().await;
    if let Some(existing) = session_from_thread_index(&state, thread_id)?.cloned() {
        if existing.tenant_id != tenant_id {
            return Err(WebError::Forbidden);
        }
        ensure_session_active(&existing)?;
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
        lifecycle: SessionLifecycle::Active,
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
    ensure_tenant_owns_session(&session, tenant_id)?;
    ensure_session_active(&session)?;
    Ok(session)
}

pub(super) async fn begin_close_owned_session(
    state: &Arc<RwLock<WebState>>,
    tenant_id: &str,
    session_id: &str,
) -> Result<SessionRecord, WebError> {
    let mut state = state.write().await;
    let record = state
        .sessions
        .get_mut(session_id)
        .ok_or(WebError::InvalidSession)?;
    ensure_tenant_owns_session(record, tenant_id)?;
    ensure_session_active(record)?;
    record.lifecycle = SessionLifecycle::Closing;
    Ok(record.clone())
}

pub(super) async fn finalize_close_owned_session(
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
    ensure_tenant_owns_session(&session, tenant_id)?;
    if session.lifecycle != SessionLifecycle::Closing {
        return Err(WebError::Internal(
            "session close finalization requires closing lifecycle".to_owned(),
        ));
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

pub(super) async fn rollback_close_owned_session(
    state: &Arc<RwLock<WebState>>,
    tenant_id: &str,
    session_id: &str,
) -> Result<(), WebError> {
    let mut state = state.write().await;
    let session = state
        .sessions
        .get_mut(session_id)
        .ok_or(WebError::InvalidSession)?;
    ensure_tenant_owns_session(session, tenant_id)?;
    if session.lifecycle == SessionLifecycle::Closing {
        session.lifecycle = SessionLifecycle::Active;
    }
    Ok(())
}

pub(super) fn new_session_id() -> String {
    format!("sess_{}", Uuid::new_v4())
}

fn session_from_thread_index<'a>(
    state: &'a WebState,
    thread_id: &str,
) -> Result<Option<&'a SessionRecord>, WebError> {
    let Some(existing_session_id) = state.thread_to_session.get(thread_id) else {
        return Ok(None);
    };
    let existing = state
        .sessions
        .get(existing_session_id)
        .ok_or_else(|| WebError::Internal(THREAD_INDEX_INCONSISTENT.to_owned()))?;
    Ok(Some(existing))
}

fn ensure_tenant_owns_session(session: &SessionRecord, tenant_id: &str) -> Result<(), WebError> {
    if session.tenant_id == tenant_id {
        return Ok(());
    }
    Err(WebError::Forbidden)
}

fn ensure_session_active(session: &SessionRecord) -> Result<(), WebError> {
    if session.lifecycle == SessionLifecycle::Active {
        return Ok(());
    }
    Err(WebError::SessionClosing)
}
