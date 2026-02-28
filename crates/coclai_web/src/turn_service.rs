use std::sync::Arc;

use coclai_runtime::turn_output::parse_turn_id;
use tokio::sync::RwLock;

use super::state;
use super::{wire, CreateTurnRequest, CreateTurnResponse, WebError, WebPluginAdapter};

pub(super) async fn create_turn(
    adapter: &Arc<dyn WebPluginAdapter>,
    state: &Arc<RwLock<state::WebState>>,
    tenant_id: &str,
    session_id: &str,
    request: CreateTurnRequest,
) -> Result<CreateTurnResponse, WebError> {
    let session = state::load_owned_session(state, tenant_id, session_id).await?;
    let params = wire::normalize_turn_start_params(&session.thread_id, &request.task)?;
    let result = adapter.turn_start(params).await?;
    let turn_id = parse_turn_id(&result).ok_or_else(|| {
        WebError::Internal(format!("turn/start missing turn id in result: {result}"))
    })?;
    Ok(CreateTurnResponse { turn_id })
}
