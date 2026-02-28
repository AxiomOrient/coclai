use std::sync::Arc;

use coclai_runtime::api::ThreadStartParams;
use tokio::sync::RwLock;

use super::state;
use super::{
    CloseSessionResponse, CreateSessionRequest, CreateSessionResponse, WebAdapterConfig, WebError,
    WebPluginAdapter,
};

pub(super) async fn create_session(
    adapter: &Arc<dyn WebPluginAdapter>,
    state: &Arc<RwLock<state::WebState>>,
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
    let thread_id = match request.thread_id.as_deref() {
        Some(thread_id) => {
            let resumed_thread_id = adapter.thread_resume(thread_id, thread_params).await?;
            if resumed_thread_id != thread_id {
                return Err(WebError::Internal(format!(
                    "thread/resume returned mismatched thread id: requested={thread_id} actual={resumed_thread_id}"
                )));
            }
            resumed_thread_id
        }
        None => adapter.thread_start(thread_params).await?,
    };

    state::register_session(state, config, tenant_id, &request.artifact_id, &thread_id).await
}

pub(super) async fn close_session(
    adapter: &Arc<dyn WebPluginAdapter>,
    state: &Arc<RwLock<state::WebState>>,
    tenant_id: &str,
    session_id: &str,
) -> Result<CloseSessionResponse, WebError> {
    let session = state::close_owned_session(state, tenant_id, session_id).await?;
    let archived = adapter.thread_archive(&session.thread_id).await.is_ok();
    Ok(CloseSessionResponse {
        thread_id: session.thread_id,
        archived,
    })
}
