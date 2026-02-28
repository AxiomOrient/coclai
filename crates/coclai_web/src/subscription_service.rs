use std::sync::Arc;

use coclai_runtime::approvals::ServerRequest;
use coclai_runtime::events::Envelope;
use tokio::sync::{broadcast, RwLock};

use super::state;
use super::WebError;

pub(super) async fn subscribe_session_events(
    state: &Arc<RwLock<state::WebState>>,
    tenant_id: &str,
    session_id: &str,
) -> Result<broadcast::Receiver<Envelope>, WebError> {
    let _ = state::load_owned_session(state, tenant_id, session_id).await?;
    let sender = {
        let state = state.read().await;
        state
            .event_topics
            .get(session_id)
            .cloned()
            .ok_or(WebError::InvalidSession)?
    };
    Ok(sender.subscribe())
}

pub(super) async fn subscribe_session_approvals(
    state: &Arc<RwLock<state::WebState>>,
    tenant_id: &str,
    session_id: &str,
) -> Result<broadcast::Receiver<ServerRequest>, WebError> {
    let _ = state::load_owned_session(state, tenant_id, session_id).await?;
    let sender = {
        let state = state.read().await;
        state
            .approval_topics
            .get(session_id)
            .cloned()
            .ok_or(WebError::InvalidSession)?
    };
    Ok(sender.subscribe())
}
