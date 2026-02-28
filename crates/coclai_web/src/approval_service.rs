use std::sync::Arc;

use tokio::sync::RwLock;

use super::state;
use super::{ApprovalResponsePayload, WebError, WebPluginAdapter};

pub(super) async fn post_approval(
    adapter: &Arc<dyn WebPluginAdapter>,
    state: &Arc<RwLock<state::WebState>>,
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
