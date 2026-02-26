use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::sync::oneshot;
use tokio::time::timeout;

use crate::errors::{RpcError, RuntimeError};

use super::{state_projection::state_clear_pending_server_requests, RuntimeInner};

pub(super) async fn call_raw_inner(
    inner: &Arc<RuntimeInner>,
    method: &str,
    params: Value,
    timeout_duration: Duration,
) -> Result<Value, RpcError> {
    let outbound_tx = inner
        .outbound_tx
        .lock()
        .await
        .clone()
        .ok_or(RpcError::TransportClosed)?;

    let rpc_id = inner.next_rpc_id.fetch_add(1, Ordering::Relaxed);
    let (pending_tx, pending_rx) = oneshot::channel();
    inner.pending.lock().await.insert(rpc_id, pending_tx);
    inner.metrics.inc_pending_rpc();

    let request = json!({
        "id": rpc_id,
        "method": method,
        "params": params
    });
    if outbound_tx.send(request).await.is_err() {
        if inner.pending.lock().await.remove(&rpc_id).is_some() {
            inner.metrics.dec_pending_rpc();
        }
        return Err(RpcError::TransportClosed);
    }

    match timeout(timeout_duration, pending_rx).await {
        Ok(Ok(result)) => result,
        Ok(Err(_)) => Err(RpcError::TransportClosed),
        Err(_) => {
            if inner.pending.lock().await.remove(&rpc_id).is_some() {
                inner.metrics.dec_pending_rpc();
            }
            Err(RpcError::Timeout)
        }
    }
}

pub(super) async fn notify_raw_inner(
    inner: &Arc<RuntimeInner>,
    method: &str,
    params: Value,
) -> Result<(), RuntimeError> {
    let outbound_tx = inner
        .outbound_tx
        .lock()
        .await
        .clone()
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
    let mut pending = inner.pending.lock().await;
    for (_, tx) in pending.drain() {
        let _ = tx.send(Err(RpcError::TransportClosed));
    }
    drop(pending);
    inner.metrics.set_pending_rpc_count(0);

    inner.pending_server_requests.lock().await.clear();
    inner.metrics.set_pending_server_request_count(0);
    state_clear_pending_server_requests(inner);
}
