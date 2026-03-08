use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::sync::RwLock;

use crate::plugin::{HookContext, HookReport};
use arc_swap::ArcSwapOption;
use serde_json::Value;
use tokio::sync::{broadcast, mpsc, oneshot, Mutex, Notify};
use tokio::task::JoinHandle;
use tokio::time::Duration;

#[cfg(test)]
use crate::runtime::approvals::TimeoutAction;
use crate::runtime::approvals::{ServerRequest, ServerRequestConfig};
use crate::runtime::errors::{RpcError, RuntimeError};
use crate::runtime::events::{Envelope, JsonRpcId};
use crate::runtime::hooks::{HookKernel, PreHookDecision, RuntimeHookConfig};
use crate::runtime::metrics::{RuntimeMetrics, RuntimeMetricsSnapshot};
use crate::runtime::runtime_validation::validate_runtime_capacities;
#[cfg(test)]
use crate::runtime::state::ConnectionState;
use crate::runtime::state::{RuntimeState, StateProjectionLimits};
use crate::runtime::transport::{StdioProcessSpec, StdioTransport, StdioTransportConfig};

type PendingResult = Result<Value, RpcError>;

mod approval;
mod config;
mod dispatch;
pub(crate) mod io_policy;
mod lifecycle;
mod rpc;
mod rpc_io;
mod state_projection;
mod supervisor;

pub use config::{RestartPolicy, RuntimeConfig, SupervisorConfig};
use dispatch::event_sink_loop;
use lifecycle::{shutdown_runtime, spawn_connection_generation};
use state_projection::state_snapshot_arc;
use supervisor::start_supervisor_task;

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

        validate_runtime_capacities(
            live_channel_capacity,
            server_request_channel_capacity,
            event_sink.is_some(),
            event_sink_channel_capacity,
            rpc_response_timeout,
        )?;
        crate::runtime::runtime_validation::validate_state_projection_limits(
            &state_projection_limits,
        )?;

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

    pub async fn shutdown(&self) -> Result<(), RuntimeError> {
        shutdown_runtime(&self.inner).await
    }
}

use super::now_millis;

#[cfg(test)]
mod tests;
