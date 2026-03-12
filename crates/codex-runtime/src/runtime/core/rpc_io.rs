use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::sync::oneshot;
use tokio::time::timeout;

use crate::runtime::errors::{RpcError, RuntimeError};

use super::io_policy::{build_rpc_request, project_pending_rpc_outcome, PendingRpcOutcome};
use super::{state_projection::state_clear_pending_server_requests, RuntimeInner};

pub(super) async fn call_raw_inner(
    inner: &Arc<RuntimeInner>,
    method: &str,
    params: Value,
    timeout_duration: Duration,
) -> Result<Value, RpcError> {
    let outbound_tx = inner
        .io
        .outbound_tx
        .load_full()
        .ok_or(RpcError::TransportClosed)?;

    let rpc_id = inner.counters.next_rpc_id.fetch_add(1, Ordering::Relaxed);
    let (pending_tx, pending_rx) = oneshot::channel();
    inner.io.pending.lock().await.insert(rpc_id, pending_tx);
    inner.metrics.inc_pending_rpc();
    let mut pending_guard = PendingRpcGuard::new(inner, rpc_id);

    let request = build_rpc_request(rpc_id, method, params);
    if outbound_tx.send(request).await.is_err() {
        clear_pending_rpc(inner, rpc_id).await;
        pending_guard.disarm();
        return Err(RpcError::TransportClosed);
    }

    let result = match project_pending_rpc_outcome(timeout(timeout_duration, pending_rx).await) {
        PendingRpcOutcome::Ready(result) => result,
        PendingRpcOutcome::Timeout => {
            clear_pending_rpc(inner, rpc_id).await;
            Err(RpcError::Timeout)
        }
    };
    pending_guard.disarm();
    result
}

pub(super) async fn notify_raw_inner(
    inner: &Arc<RuntimeInner>,
    method: &str,
    params: Value,
) -> Result<(), RuntimeError> {
    let outbound_tx = inner
        .io
        .outbound_tx
        .load_full()
        .ok_or(RuntimeError::TransportClosed)?;

    let notification = json!({
        "method": method,
        "params": params
    });
    outbound_tx
        .send(notification)
        .await
        .map_err(|_| RuntimeError::TransportClosed)
}

pub(super) async fn resolve_transport_closed_pending(inner: &Arc<RuntimeInner>) {
    let mut pending = inner.io.pending.lock().await;
    for (_, tx) in pending.drain() {
        let _ = tx.send(Err(RpcError::TransportClosed));
    }
    drop(pending);
    inner.metrics.set_pending_rpc_count(0);

    inner.io.pending_server_requests.lock().await.clear();
    inner.metrics.set_pending_server_request_count(0);
    state_clear_pending_server_requests(inner);
}

async fn clear_pending_rpc(inner: &Arc<RuntimeInner>, rpc_id: u64) {
    if inner.io.pending.lock().await.remove(&rpc_id).is_some() {
        inner.metrics.dec_pending_rpc();
    }
}

struct PendingRpcGuard {
    inner: Arc<RuntimeInner>,
    rpc_id: u64,
    armed: bool,
}

impl PendingRpcGuard {
    fn new(inner: &Arc<RuntimeInner>, rpc_id: u64) -> Self {
        Self {
            inner: inner.clone(),
            rpc_id,
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for PendingRpcGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let inner = self.inner.clone();
        let rpc_id = self.rpc_id;
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                clear_pending_rpc(&inner, rpc_id).await;
            });
        }
    }
}
