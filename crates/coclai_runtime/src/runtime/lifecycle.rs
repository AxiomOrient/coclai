use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use serde_json::Value;

use crate::errors::RuntimeError;
use crate::state::ConnectionState;
use crate::transport::StdioTransport;

use super::dispatch::dispatcher_loop;
use super::rpc_io::{call_raw_inner, notify_raw_inner, resolve_transport_closed_pending};
use super::state_projection::state_set_connection;
use super::RuntimeInner;

pub(super) async fn spawn_connection_generation(
    inner: &Arc<RuntimeInner>,
    generation: u64,
) -> Result<(), RuntimeError> {
    if inner.shutting_down.load(Ordering::Acquire) {
        return Err(RuntimeError::TransportClosed);
    }

    state_set_connection(inner, ConnectionState::Starting);
    set_initialize_result(inner, None);

    let mut transport = StdioTransport::spawn(inner.process.clone(), inner.transport_cfg).await?;
    let read_rx = transport.take_read_rx()?;
    let outbound_tx = transport.write_tx();

    {
        let mut outbound_guard = inner.outbound_tx.lock().await;
        outbound_guard.replace(outbound_tx);
    }

    {
        let mut transport_guard = inner.transport.lock().await;
        transport_guard.replace(transport);
    }

    let dispatcher_inner = Arc::clone(inner);
    let dispatcher_task = tokio::spawn(dispatcher_loop(dispatcher_inner, read_rx));
    inner.dispatcher_task.lock().await.replace(dispatcher_task);

    state_set_connection(inner, ConnectionState::Handshaking);
    let initialize_result = match call_raw_inner(
        inner,
        "initialize",
        inner.initialize_params.clone(),
        inner.rpc_response_timeout,
    )
    .await
    {
        Ok(value) => value,
        Err(err) => {
            detach_generation(inner).await;
            return Err(RuntimeError::Internal(format!(
                "initialize handshake failed: {err}"
            )));
        }
    };
    if let Err(err) = notify_raw_inner(inner, "initialized", json!({})).await {
        detach_generation(inner).await;
        return Err(err);
    }
    set_initialize_result(inner, Some(initialize_result));

    inner.generation.store(generation, Ordering::Release);
    inner.initialized.store(true, Ordering::Release);
    state_set_connection(inner, ConnectionState::Running { generation });
    Ok(())
}

pub(super) async fn detach_generation(inner: &Arc<RuntimeInner>) {
    {
        let mut outbound = inner.outbound_tx.lock().await;
        outbound.take();
    }

    if let Some(transport) = inner.transport.lock().await.take() {
        let flush_timeout = Duration::from_millis(inner.supervisor_cfg.shutdown_flush_timeout_ms);
        let terminate_grace =
            Duration::from_millis(inner.supervisor_cfg.shutdown_terminate_grace_ms);
        let _ = transport
            .terminate_and_join(flush_timeout, terminate_grace)
            .await;
    }

    if let Some(dispatcher_task) = inner.dispatcher_task.lock().await.take() {
        let _ = dispatcher_task.await;
    }

    resolve_transport_closed_pending(inner).await;
}

fn set_initialize_result(inner: &Arc<RuntimeInner>, result: Option<Value>) {
    match inner.initialize_result.write() {
        Ok(mut guard) => {
            *guard = result;
        }
        Err(poisoned) => {
            *poisoned.into_inner() = result;
        }
    }
}
