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
    if inner.counters.shutting_down.load(Ordering::Acquire) {
        return Err(RuntimeError::TransportClosed);
    }

    state_set_connection(inner, ConnectionState::Starting);
    set_initialize_result(inner, None);

    let mut transport =
        StdioTransport::spawn(inner.spec.process.clone(), inner.spec.transport_cfg).await?;
    let read_rx = transport.take_read_rx()?;
    let outbound_tx = transport.write_tx()?;

    inner.io.outbound_tx.store(Some(Arc::new(outbound_tx)));

    {
        let mut transport_guard = inner.tasks.transport.lock().await;
        transport_guard.replace(transport);
    }

    let dispatcher_inner = Arc::clone(inner);
    let dispatcher_task = tokio::spawn(dispatcher_loop(dispatcher_inner, read_rx));
    inner
        .tasks
        .dispatcher_task
        .lock()
        .await
        .replace(dispatcher_task);

    state_set_connection(inner, ConnectionState::Handshaking);
    let initialize_result = match call_raw_inner(
        inner,
        "initialize",
        inner.spec.initialize_params.clone(),
        inner.spec.rpc_response_timeout,
    )
    .await
    {
        Ok(value) => value,
        Err(err) => {
            if let Err(detach_err) = detach_generation(inner).await {
                return Err(RuntimeError::Internal(format!(
                    "initialize handshake failed: {err}; detach failed: {detach_err}"
                )));
            }
            return Err(RuntimeError::Internal(format!(
                "initialize handshake failed: {err}"
            )));
        }
    };
    if let Err(err) = notify_raw_inner(inner, "initialized", json!({})).await {
        if let Err(detach_err) = detach_generation(inner).await {
            return Err(RuntimeError::Internal(format!(
                "initialized notify failed: {err}; detach failed: {detach_err}"
            )));
        }
        return Err(err);
    }
    set_initialize_result(inner, Some(initialize_result));

    inner
        .counters
        .generation
        .store(generation, Ordering::Release);
    inner.counters.initialized.store(true, Ordering::Release);
    state_set_connection(inner, ConnectionState::Running { generation });
    Ok(())
}

pub(super) async fn detach_generation(inner: &Arc<RuntimeInner>) -> Result<(), RuntimeError> {
    teardown_generation(inner, TeardownContext::Detach).await
}

pub(super) async fn shutdown_runtime(inner: &Arc<RuntimeInner>) -> Result<(), RuntimeError> {
    inner.counters.shutting_down.store(true, Ordering::Release);
    inner.counters.initialized.store(false, Ordering::Release);
    state_set_connection(inner, ConnectionState::ShuttingDown);
    inner.io.shutdown_signal.notify_waiters();

    teardown_generation(inner, TeardownContext::Shutdown).await?;

    if let Some(supervisor_task) = inner.tasks.supervisor_task.lock().await.take() {
        if let Err(err) = supervisor_task.await {
            state_set_connection(inner, ConnectionState::Dead);
            return Err(RuntimeError::Internal(format!(
                "supervisor task join failed during shutdown: {err}"
            )));
        }
    }

    if let Some(event_sink_task) = inner.tasks.event_sink_task.lock().await.take() {
        event_sink_task.abort();
        match event_sink_task.await {
            Ok(()) => {}
            Err(err) if err.is_cancelled() => {}
            Err(err) => {
                state_set_connection(inner, ConnectionState::Dead);
                return Err(RuntimeError::Internal(format!(
                    "event sink task join failed during shutdown: {err}"
                )));
            }
        }
    }

    state_set_connection(inner, ConnectionState::Dead);
    Ok(())
}

fn set_initialize_result(inner: &Arc<RuntimeInner>, result: Option<Value>) {
    match inner.snapshots.initialize_result.write() {
        Ok(mut guard) => {
            *guard = result;
        }
        Err(poisoned) => {
            *poisoned.into_inner() = result;
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TeardownContext {
    Detach,
    Shutdown,
}

impl TeardownContext {
    fn dispatcher_join_phase(self) -> &'static str {
        match self {
            TeardownContext::Detach => "detach",
            TeardownContext::Shutdown => "shutdown",
        }
    }
}

async fn teardown_generation(
    inner: &Arc<RuntimeInner>,
    context: TeardownContext,
) -> Result<(), RuntimeError> {
    inner.io.outbound_tx.store(None);

    if let Some(transport) = inner.tasks.transport.lock().await.take() {
        let flush_timeout =
            Duration::from_millis(inner.spec.supervisor_cfg.shutdown_flush_timeout_ms);
        let terminate_grace =
            Duration::from_millis(inner.spec.supervisor_cfg.shutdown_terminate_grace_ms);
        transport
            .terminate_and_join(flush_timeout, terminate_grace)
            .await?;
    }

    if let Some(dispatcher_task) = inner.tasks.dispatcher_task.lock().await.take() {
        let phase = context.dispatcher_join_phase();
        dispatcher_task.await.map_err(|err| {
            RuntimeError::Internal(format!("dispatcher task join failed during {phase}: {err}"))
        })?;
    }

    resolve_transport_closed_pending(inner).await;
    Ok(())
}
