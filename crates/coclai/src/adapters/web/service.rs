use std::sync::Arc;

use tokio::sync::RwLock;

use crate::plugin::PluginContractVersion;
use crate::runtime::rpc_contract::methods as events;

use super::state::WebState;
use super::{handlers, WebAdapterConfig, WebError, WebPluginAdapter, WebRuntimeStreams};

pub(super) async fn prepare_spawn(
    adapter: &Arc<dyn WebPluginAdapter>,
    config: &WebAdapterConfig,
) -> Result<WebRuntimeStreams, WebError> {
    validate_web_adapter_config(config)?;
    ensure_adapter_contract_compatible(adapter.as_ref())?;
    adapter.take_streams().await
}

pub(super) fn spawn_routing_tasks(
    adapter: Arc<dyn WebPluginAdapter>,
    state: Arc<RwLock<WebState>>,
    streams: WebRuntimeStreams,
) -> Vec<tokio::task::AbortHandle> {
    let WebRuntimeStreams {
        mut request_rx,
        mut live_rx,
    } = streams;
    let adapter_for_events = Arc::clone(&adapter);
    let adapter_for_approvals = adapter;
    let state_for_events = Arc::clone(&state);
    let state_for_approvals = state;

    let events_task = tokio::spawn(async move {
        loop {
            match live_rx.recv().await {
                Ok(envelope) => {
                    handle_live_event(&state_for_events, &adapter_for_events, envelope).await;
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
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

    vec![events_task.abort_handle(), approvals_task.abort_handle()]
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
    state: &Arc<RwLock<WebState>>,
    adapter: &Arc<dyn WebPluginAdapter>,
    envelope: crate::runtime::events::Envelope,
) {
    let should_prune = envelope.method.as_deref() == Some(events::APPROVAL_ACK);
    handlers::route_session_event(state, envelope).await;
    if should_prune {
        handlers::prune_stale_approval_index(state, adapter).await;
    }
}
