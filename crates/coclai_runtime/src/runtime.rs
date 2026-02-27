use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::sync::RwLock;
use std::time::{SystemTime, UNIX_EPOCH};

use arc_swap::ArcSwapOption;
use coclai_plugin_core::{HookContext, HookReport};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Value};
use tokio::sync::{broadcast, mpsc, oneshot, Mutex, Notify};
use tokio::task::JoinHandle;
use tokio::time::Duration;
#[cfg(test)]
use uuid::Uuid;

#[cfg(test)]
use crate::approvals::TimeoutAction;
use crate::approvals::{ServerRequest, ServerRequestConfig};
use crate::errors::{RpcError, RuntimeError};
use crate::events::{Envelope, JsonRpcId};
use crate::hooks::{HookKernel, PreHookDecision, RuntimeHookConfig};
use crate::metrics::{RuntimeMetrics, RuntimeMetricsSnapshot};
use crate::rpc_contract::{validate_rpc_request, validate_rpc_response, RpcValidationMode};
use crate::runtime_schema::{validate_runtime_capacities, validate_schema_guard};
use crate::sink::EventSink;
use crate::state::{RuntimeState, StateProjectionLimits};
use crate::transport::{StdioProcessSpec, StdioTransport, StdioTransportConfig};
#[cfg(test)]
use crate::ConnectionState;

type PendingResult = Result<Value, RpcError>;

mod dispatch;
mod lifecycle;
mod rpc_io;
mod state_projection;
mod supervisor;

use dispatch::{
    event_sink_loop, send_rpc_error, send_rpc_result, validate_server_request_result_payload,
};
use lifecycle::{shutdown_runtime, spawn_connection_generation};
use rpc_io::{call_raw_inner, notify_raw_inner};
use state_projection::state_remove_pending_server_request;
use state_projection::state_snapshot_arc;
use supervisor::start_supervisor_task;

#[derive(Clone)]
pub struct RuntimeConfig {
    pub process: StdioProcessSpec,
    pub schema_guard: SchemaGuardConfig,
    pub hooks: RuntimeHookConfig,
    pub transport: StdioTransportConfig,
    pub supervisor: SupervisorConfig,
    pub rpc_response_timeout: Duration,
    pub server_requests: ServerRequestConfig,
    pub initialize_params: Value,
    pub live_channel_capacity: usize,
    pub server_request_channel_capacity: usize,
    pub event_sink: Option<Arc<dyn EventSink>>,
    pub event_sink_channel_capacity: usize,
    pub state_projection_limits: StateProjectionLimits,
}

impl RuntimeConfig {
    pub fn new(process: StdioProcessSpec, schema_guard: SchemaGuardConfig) -> Self {
        Self {
            process,
            schema_guard,
            hooks: RuntimeHookConfig::default(),
            transport: StdioTransportConfig::default(),
            supervisor: SupervisorConfig::default(),
            rpc_response_timeout: Duration::from_secs(30),
            server_requests: ServerRequestConfig::default(),
            initialize_params: json!({
                "clientInfo": {
                    "name": "coclai_runtime",
                    "title": "coclai_runtime",
                    "version": env!("CARGO_PKG_VERSION")
                },
                "capabilities": {}
            }),
            live_channel_capacity: 1024,
            server_request_channel_capacity: 128,
            event_sink: None,
            event_sink_channel_capacity: 1024,
            state_projection_limits: StateProjectionLimits::default(),
        }
    }

    /// Override lifecycle hook configuration.
    /// Allocation: O(h), h = hook count in config clone/move.
    pub fn with_hooks(mut self, hooks: RuntimeHookConfig) -> Self {
        self.hooks = hooks;
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SchemaGuardConfig {
    pub active_schema_dir: PathBuf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RestartPolicy {
    Never,
    OnCrash {
        max_restarts: u32,
        base_backoff_ms: u64,
        max_backoff_ms: u64,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SupervisorConfig {
    pub restart: RestartPolicy,
    pub shutdown_flush_timeout_ms: u64,
    pub shutdown_terminate_grace_ms: u64,
}

impl Default for SupervisorConfig {
    fn default() -> Self {
        Self {
            restart: RestartPolicy::Never,
            shutdown_flush_timeout_ms: 500,
            shutdown_terminate_grace_ms: 750,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PendingServerRequestEntry {
    rpc_id: JsonRpcId,
    rpc_key: String,
    method: String,
    created_at_millis: i64,
    deadline_millis: i64,
}

struct RuntimeCounters {
    initialized: AtomicBool,
    shutting_down: AtomicBool,
    generation: AtomicU64,
    next_rpc_id: AtomicU64,
    next_seq: AtomicU64,
}

struct RuntimeSpec {
    process: StdioProcessSpec,
    transport_cfg: StdioTransportConfig,
    initialize_params: Value,
    supervisor_cfg: SupervisorConfig,
    rpc_response_timeout: Duration,
    server_request_cfg: ServerRequestConfig,
    state_projection_limits: StateProjectionLimits,
}

struct RuntimeIo {
    pending: Mutex<HashMap<u64, oneshot::Sender<PendingResult>>>,
    outbound_tx: ArcSwapOption<mpsc::Sender<Value>>,
    live_tx: broadcast::Sender<Envelope>,
    pending_server_requests: Mutex<HashMap<String, PendingServerRequestEntry>>,
    server_request_tx: mpsc::Sender<ServerRequest>,
    server_request_rx: Mutex<Option<mpsc::Receiver<ServerRequest>>>,
    event_sink_tx: Option<mpsc::Sender<Envelope>>,
    transport_closed_signal: Notify,
    shutdown_signal: Notify,
}

struct RuntimeTasks {
    event_sink_task: Mutex<Option<JoinHandle<()>>>,
    supervisor_task: Mutex<Option<JoinHandle<()>>>,
    dispatcher_task: Mutex<Option<JoinHandle<()>>>,
    transport: Mutex<Option<StdioTransport>>,
}

struct RuntimeSnapshots {
    state: RwLock<Arc<RuntimeState>>,
    initialize_result: RwLock<Option<Value>>,
}

#[derive(Clone)]
pub struct Runtime {
    inner: Arc<RuntimeInner>,
}

struct RuntimeInner {
    counters: RuntimeCounters,
    spec: RuntimeSpec,
    io: RuntimeIo,
    tasks: RuntimeTasks,
    snapshots: RuntimeSnapshots,
    metrics: Arc<RuntimeMetrics>,
    hooks: HookKernel,
}

impl Runtime {
    pub async fn spawn_local(cfg: RuntimeConfig) -> Result<Self, RuntimeError> {
        let RuntimeConfig {
            process,
            schema_guard,
            hooks,
            transport,
            supervisor,
            rpc_response_timeout,
            server_requests,
            initialize_params,
            live_channel_capacity,
            server_request_channel_capacity,
            event_sink,
            event_sink_channel_capacity,
            state_projection_limits,
        } = cfg;

        validate_schema_guard(&schema_guard)?;
        validate_runtime_capacities(
            live_channel_capacity,
            server_request_channel_capacity,
            event_sink.is_some(),
            event_sink_channel_capacity,
            rpc_response_timeout,
        )?;
        crate::runtime_schema::validate_state_projection_limits(&state_projection_limits)?;

        let (live_tx, _) = broadcast::channel(live_channel_capacity);
        let (server_request_tx, server_request_rx) = mpsc::channel(server_request_channel_capacity);
        let metrics = Arc::new(RuntimeMetrics::new(now_millis()));
        let (event_sink_tx, event_sink_task) = match event_sink {
            Some(sink) => {
                let (tx, rx) = mpsc::channel(event_sink_channel_capacity);
                let task = tokio::spawn(event_sink_loop(sink, Arc::clone(&metrics), rx));
                (Some(tx), Some(task))
            }
            None => (None, None),
        };

        let runtime = Self {
            inner: Arc::new(RuntimeInner {
                counters: RuntimeCounters {
                    initialized: AtomicBool::new(false),
                    shutting_down: AtomicBool::new(false),
                    generation: AtomicU64::new(0),
                    next_rpc_id: AtomicU64::new(1),
                    next_seq: AtomicU64::new(0),
                },
                spec: RuntimeSpec {
                    process,
                    transport_cfg: transport,
                    initialize_params,
                    supervisor_cfg: supervisor,
                    rpc_response_timeout,
                    server_request_cfg: server_requests,
                    state_projection_limits,
                },
                io: RuntimeIo {
                    pending: Mutex::new(HashMap::new()),
                    outbound_tx: ArcSwapOption::new(None),
                    live_tx,
                    pending_server_requests: Mutex::new(HashMap::new()),
                    server_request_tx,
                    server_request_rx: Mutex::new(Some(server_request_rx)),
                    event_sink_tx,
                    transport_closed_signal: Notify::new(),
                    shutdown_signal: Notify::new(),
                },
                tasks: RuntimeTasks {
                    event_sink_task: Mutex::new(event_sink_task),
                    supervisor_task: Mutex::new(None),
                    dispatcher_task: Mutex::new(None),
                    transport: Mutex::new(None),
                },
                snapshots: RuntimeSnapshots {
                    state: RwLock::new(Arc::new(RuntimeState::default())),
                    initialize_result: RwLock::new(None),
                },
                metrics,
                hooks: HookKernel::new(hooks),
            }),
        };

        spawn_connection_generation(&runtime.inner, 0).await?;
        start_supervisor_task(&runtime.inner).await;

        Ok(runtime)
    }

    pub fn subscribe_live(&self) -> broadcast::Receiver<Envelope> {
        self.inner.io.live_tx.subscribe()
    }

    pub fn is_initialized(&self) -> bool {
        self.inner.counters.initialized.load(Ordering::Acquire)
    }

    pub fn state_snapshot(&self) -> Arc<RuntimeState> {
        state_snapshot_arc(&self.inner)
    }

    pub fn initialize_result_snapshot(&self) -> Option<Value> {
        match self.inner.snapshots.initialize_result.read() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    pub fn server_user_agent(&self) -> Option<String> {
        self.initialize_result_snapshot()
            .and_then(|value| value.get("userAgent").cloned())
            .and_then(|value| value.as_str().map(ToOwned::to_owned))
    }

    pub fn metrics_snapshot(&self) -> RuntimeMetricsSnapshot {
        self.inner.metrics.snapshot(now_millis())
    }

    /// Return latest hook report snapshot (last completed hook-enabled call wins).
    /// Allocation: clones report payload. Complexity: O(i), i = issue count.
    pub fn hook_report_snapshot(&self) -> HookReport {
        self.inner.hooks.report_snapshot()
    }

    /// Register additional lifecycle hooks into running runtime.
    /// Duplicate hook names are ignored.
    /// Allocation: O(n) for dedup snapshot. Complexity: O(n + m), n=existing, m=incoming.
    pub fn register_hooks(&self, hooks: RuntimeHookConfig) {
        self.inner.hooks.register(hooks);
    }

    pub(crate) fn hooks_enabled(&self) -> bool {
        self.inner.hooks.is_enabled()
    }

    pub(crate) fn hooks_enabled_with(&self, scoped_hooks: Option<&RuntimeHookConfig>) -> bool {
        self.hooks_enabled() || scoped_hooks.is_some_and(|hooks| !hooks.is_empty())
    }

    pub(crate) fn next_hook_correlation_id(&self) -> String {
        let seq = self.inner.counters.next_seq.fetch_add(1, Ordering::AcqRel) + 1;
        format!("hk-{seq}")
    }

    pub(crate) fn publish_hook_report(&self, report: HookReport) {
        self.inner.hooks.set_latest_report(report);
    }

    pub(crate) async fn run_pre_hooks_with(
        &self,
        ctx: &HookContext,
        report: &mut HookReport,
        scoped_hooks: Option<&RuntimeHookConfig>,
    ) -> Vec<PreHookDecision> {
        self.inner
            .hooks
            .run_pre_with(ctx, report, scoped_hooks)
            .await
    }

    pub(crate) async fn run_post_hooks_with(
        &self,
        ctx: &HookContext,
        report: &mut HookReport,
        scoped_hooks: Option<&RuntimeHookConfig>,
    ) {
        self.inner
            .hooks
            .run_post_with(ctx, report, scoped_hooks)
            .await;
    }

    pub async fn take_server_request_rx(
        &self,
    ) -> Result<mpsc::Receiver<ServerRequest>, RuntimeError> {
        self.inner
            .io
            .server_request_rx
            .lock()
            .await
            .take()
            .ok_or(RuntimeError::ServerRequestReceiverTaken)
    }

    pub async fn respond_approval_ok(
        &self,
        approval_id: &str,
        result: Value,
    ) -> Result<(), RuntimeError> {
        let entry = {
            let mut guard = self.inner.io.pending_server_requests.lock().await;
            let entry = guard.get(approval_id).cloned().ok_or_else(|| {
                RuntimeError::Internal(format!("approval id not found: {approval_id}"))
            })?;
            validate_server_request_result_payload(&entry.method, &result)?;
            guard.remove(approval_id);
            entry
        };
        self.inner.metrics.dec_pending_server_request();
        state_remove_pending_server_request(&self.inner, &entry.rpc_key);
        send_rpc_result(&self.inner, &entry.rpc_id, result).await
    }

    pub async fn respond_approval_err(
        &self,
        approval_id: &str,
        err: crate::errors::RpcErrorObject,
    ) -> Result<(), RuntimeError> {
        let entry = {
            let mut guard = self.inner.io.pending_server_requests.lock().await;
            guard.remove(approval_id).ok_or_else(|| {
                RuntimeError::Internal(format!("approval id not found: {approval_id}"))
            })?
        };
        self.inner.metrics.dec_pending_server_request();
        state_remove_pending_server_request(&self.inner, &entry.rpc_key);
        send_rpc_error(
            &self.inner,
            &entry.rpc_id,
            json!({
                "code": err.code,
                "message": err.message,
                "data": err.data
            }),
        )
        .await
    }

    pub async fn call_raw(&self, method: &str, params: Value) -> Result<Value, RpcError> {
        self.call_raw_internal(method, params, true, self.inner.spec.rpc_response_timeout)
            .await
    }

    /// JSON-RPC call with contract validation for known methods.
    /// Validation covers request params before send and result shape after receive.
    pub async fn call_validated(&self, method: &str, params: Value) -> Result<Value, RpcError> {
        self.call_validated_with_mode(method, params, RpcValidationMode::KnownMethods)
            .await
    }

    /// JSON-RPC call with explicit validation mode.
    pub async fn call_validated_with_mode(
        &self,
        method: &str,
        params: Value,
        mode: RpcValidationMode,
    ) -> Result<Value, RpcError> {
        validate_rpc_request(method, &params, mode)?;
        let result = self
            .call_raw_internal(method, params, true, self.inner.spec.rpc_response_timeout)
            .await?;
        validate_rpc_response(method, &result, mode)?;
        Ok(result)
    }

    /// Typed JSON-RPC call with known-method contract validation.
    pub async fn call_typed_validated<P, R>(&self, method: &str, params: P) -> Result<R, RpcError>
    where
        P: Serialize,
        R: DeserializeOwned,
    {
        self.call_typed_validated_with_mode(method, params, RpcValidationMode::KnownMethods)
            .await
    }

    /// Typed JSON-RPC call with explicit validation mode.
    pub async fn call_typed_validated_with_mode<P, R>(
        &self,
        method: &str,
        params: P,
        mode: RpcValidationMode,
    ) -> Result<R, RpcError>
    where
        P: Serialize,
        R: DeserializeOwned,
    {
        let params_value = serde_json::to_value(params).map_err(|err| {
            RpcError::InvalidRequest(format!(
                "failed to serialize json-rpc params for {method}: {err}"
            ))
        })?;
        let result = self
            .call_validated_with_mode(method, params_value, mode)
            .await?;
        serde_json::from_value(result).map_err(|err| {
            RpcError::InvalidRequest(format!(
                "failed to deserialize json-rpc result for {method}: {err}"
            ))
        })
    }

    pub(crate) async fn call_raw_with_timeout(
        &self,
        method: &str,
        params: Value,
        timeout_duration: Duration,
    ) -> Result<Value, RpcError> {
        self.call_raw_internal(method, params, true, timeout_duration)
            .await
    }

    pub async fn notify_raw(&self, method: &str, params: Value) -> Result<(), RuntimeError> {
        self.notify_raw_internal(method, params, true).await
    }

    /// JSON-RPC notify with known-method request validation.
    pub async fn notify_validated(&self, method: &str, params: Value) -> Result<(), RuntimeError> {
        self.notify_validated_with_mode(method, params, RpcValidationMode::KnownMethods)
            .await
    }

    /// JSON-RPC notify with explicit validation mode.
    pub async fn notify_validated_with_mode(
        &self,
        method: &str,
        params: Value,
        mode: RpcValidationMode,
    ) -> Result<(), RuntimeError> {
        validate_rpc_request(method, &params, mode).map_err(|err| {
            RuntimeError::InvalidConfig(format!("invalid json-rpc notify payload: {err}"))
        })?;
        self.notify_raw_internal(method, params, true).await
    }

    /// Typed JSON-RPC notify with known-method request validation.
    pub async fn notify_typed_validated<P>(
        &self,
        method: &str,
        params: P,
    ) -> Result<(), RuntimeError>
    where
        P: Serialize,
    {
        self.notify_typed_validated_with_mode(method, params, RpcValidationMode::KnownMethods)
            .await
    }

    /// Typed JSON-RPC notify with explicit validation mode.
    pub async fn notify_typed_validated_with_mode<P>(
        &self,
        method: &str,
        params: P,
        mode: RpcValidationMode,
    ) -> Result<(), RuntimeError>
    where
        P: Serialize,
    {
        let params_value = serde_json::to_value(params).map_err(|err| {
            RuntimeError::InvalidConfig(format!(
                "invalid json-rpc notify payload: failed to serialize json-rpc params for {method}: {err}"
            ))
        })?;
        self.notify_validated_with_mode(method, params_value, mode)
            .await
    }

    pub async fn shutdown(&self) -> Result<(), RuntimeError> {
        shutdown_runtime(&self.inner).await
    }

    async fn call_raw_internal(
        &self,
        method: &str,
        params: Value,
        require_initialized: bool,
        timeout_duration: Duration,
    ) -> Result<Value, RpcError> {
        if require_initialized && !self.is_initialized() {
            return Err(RpcError::InvalidRequest(
                "runtime is not initialized".to_owned(),
            ));
        }

        call_raw_inner(&self.inner, method, params, timeout_duration).await
    }

    async fn notify_raw_internal(
        &self,
        method: &str,
        params: Value,
        require_initialized: bool,
    ) -> Result<(), RuntimeError> {
        if require_initialized && !self.is_initialized() {
            return Err(RuntimeError::NotInitialized);
        }

        notify_raw_inner(&self.inner, method, params).await
    }
}

fn now_millis() -> i64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_millis() as i64,
        Err(_) => 0,
    }
}

#[cfg(test)]
mod tests;
