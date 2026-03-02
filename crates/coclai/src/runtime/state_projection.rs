use std::sync::Arc;

use crate::approvals::PendingServerRequest;
use crate::events::Envelope;
use crate::state::ConnectionState;
use crate::state::{reduce_in_place_with_limits, RuntimeState};

use super::RuntimeInner;

pub(super) fn state_snapshot_arc(inner: &Arc<RuntimeInner>) -> Arc<RuntimeState> {
    match inner.snapshots.state.read() {
        Ok(guard) => guard.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

fn with_state_write<T>(inner: &Arc<RuntimeInner>, f: impl FnOnce(&mut RuntimeState) -> T) -> T {
    match inner.snapshots.state.write() {
        Ok(mut guard) => {
            let state = Arc::make_mut(&mut guard);
            f(state)
        }
        Err(poisoned) => {
            let mut guard = poisoned.into_inner();
            let state = Arc::make_mut(&mut guard);
            f(state)
        }
    }
}

pub(super) fn state_set_connection(inner: &Arc<RuntimeInner>, connection: ConnectionState) {
    with_state_write(inner, |state| {
        state.connection = connection;
    });
}

pub(super) fn state_apply_envelope(inner: &Arc<RuntimeInner>, envelope: &Envelope) {
    with_state_write(inner, |state| {
        reduce_in_place_with_limits(state, envelope, &inner.spec.state_projection_limits);
    });
}

pub(super) fn state_insert_pending_server_request(
    inner: &Arc<RuntimeInner>,
    rpc_id: &str,
    request: PendingServerRequest,
) {
    with_state_write(inner, |state| {
        state
            .pending_server_requests
            .insert(rpc_id.to_owned(), request);
    });
}

pub(super) fn state_remove_pending_server_request(inner: &Arc<RuntimeInner>, rpc_id: &str) {
    with_state_write(inner, |state| {
        state.pending_server_requests.remove(rpc_id);
    });
}

pub(super) fn state_clear_pending_server_requests(inner: &Arc<RuntimeInner>) {
    with_state_write(inner, |state| {
        state.pending_server_requests.clear();
    });
}
