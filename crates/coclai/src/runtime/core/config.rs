//! Runtime configuration types.
//! Pure data: no async, no runtime dependencies. Copy/Clone safe where possible.

use std::sync::Arc;

use serde_json::{json, Value};
use tokio::time::Duration;

use crate::runtime::approvals::ServerRequestConfig;
use crate::runtime::hooks::RuntimeHookConfig;
use crate::runtime::sink::EventSink;
use crate::runtime::state::StateProjectionLimits;
use crate::runtime::transport::{StdioProcessSpec, StdioTransportConfig};

// ── Supervisor ────────────────────────────────────────────────────────────

/// Restart strategy for the supervised process.
/// Copy type — zero allocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RestartPolicy {
    Never,
    OnCrash {
        max_restarts: u32,
        base_backoff_ms: u64,
        max_backoff_ms: u64,
    },
}

/// Configuration for the process supervisor lifecycle.
/// Copy type — zero allocation.
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

// ── Runtime config ────────────────────────────────────────────────────────

/// Full configuration for spawning a Runtime instance.
///
/// All fields are set with safe defaults via `RuntimeConfig::new`.
/// Override individual fields with builder methods.
/// Allocation: O(1) except `event_sink` which may hold heap state.
#[derive(Clone)]
pub struct RuntimeConfig {
    pub process: StdioProcessSpec,
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
    /// Create config with safe defaults.
    /// Allocation: one Value (JSON object). Complexity: O(1).
    pub fn new(process: StdioProcessSpec) -> Self {
        Self {
            process,
            hooks: RuntimeHookConfig::default(),
            transport: StdioTransportConfig::default(),
            supervisor: SupervisorConfig::default(),
            rpc_response_timeout: Duration::from_secs(30),
            server_requests: ServerRequestConfig::default(),
            initialize_params: json!({
                "clientInfo": {
                    "name": env!("CARGO_PKG_NAME"),
                    "title": env!("CARGO_PKG_NAME"),
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
    /// Allocation: O(h), h = hook count. Complexity: O(1).
    pub fn with_hooks(mut self, hooks: RuntimeHookConfig) -> Self {
        self.hooks = hooks;
        self
    }
}
