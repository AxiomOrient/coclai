use std::collections::HashSet;
use std::sync::Arc;

use coclai_runtime::approvals::ServerRequest;
use coclai_runtime::events::Envelope;
use coclai_runtime::rpc::extract_ids;
use serde_json::{Map, Value};
use tokio::sync::RwLock;

use super::{state::WebState, WebPluginAdapter};

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
    let wrapped = Value::Object(Map::from_iter([(
        "params".to_owned(),
        request.params.clone(),
    )]));
    extract_ids(&wrapped).thread_id
}
