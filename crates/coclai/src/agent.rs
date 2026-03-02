use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

use crate::adapters::outbound::codex_stdio::TokioCodexGateway;
use crate::adapters::outbound::memory_store::InMemoryAgentStateStore;
use crate::appserver::AppServer;
use crate::capability::CapabilityIngress;
use crate::ports::outbound::agent_state_store_port::AgentStateStorePort;
use crate::ports::outbound::codex_gateway_port::CodexGatewayPort;
use crate::ServerRequestRx;

mod authz;
mod dispatch;
mod payload_parse;
mod use_case_bridge;

/// Transport-agnostic invocation envelope.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CapabilityInvocation {
    pub capability_id: String,
    pub ingress: CapabilityIngress,
    #[serde(default)]
    pub correlation_id: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub caller_addr: Option<String>,
    #[serde(default)]
    pub auth_token: Option<String>,
    #[serde(default)]
    pub payload: Value,
}

/// Deterministic response envelope for one invocation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CapabilityResponse {
    pub capability_id: String,
    #[serde(default)]
    pub correlation_id: Option<String>,
    pub result: Value,
}

/// Minimal agent health shape for status checks.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentHealth {
    pub status: String,
    pub registry_size: usize,
    pub full_parity_gaps: usize,
    pub network_ingress_loopback_only: bool,
    pub network_ingress_token_configured: bool,
    pub workflow_registry_size: usize,
    pub appserver_registry_size: usize,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum AgentDispatchError {
    #[error("unknown capability: {0}")]
    UnknownCapability(String),
    #[error(
        "capability `{capability_id}` is not exposed on ingress `{ingress}` (status={status})"
    )]
    CapabilityNotExposed {
        capability_id: String,
        ingress: &'static str,
        status: &'static str,
    },
    #[error("invalid payload for `{capability_id}`: {message}")]
    InvalidPayload {
        capability_id: String,
        message: String,
    },
    #[error("backend failure for `{capability_id}`: {message}")]
    BackendFailure {
        capability_id: String,
        message: String,
    },
    #[error(
        "unauthorized invocation: capability `{capability_id}` ingress `{ingress}` reason `{reason}`"
    )]
    UnauthorizedInvocation {
        capability_id: String,
        ingress: &'static str,
        reason: &'static str,
    },
}

/// Security controls for non-stdio ingress calls.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSecurityPolicy {
    pub network_loopback_only: bool,
    pub required_network_token: Option<String>,
}

impl Default for AgentSecurityPolicy {
    fn default() -> Self {
        Self {
            network_loopback_only: true,
            required_network_token: std::env::var("COCLAI_AGENT_TOKEN")
                .ok()
                .filter(|token| !token.trim().is_empty()),
        }
    }
}

struct ManagedAppServer {
    appserver: AppServer,
    server_requests: Option<ServerRequestRx>,
}

#[derive(Default)]
struct AgentState {
    appservers: HashMap<String, ManagedAppServer>,
    next_workflow_id: u64,
}

/// Monolithic agent boundary. Transport adapters call this service only.
#[derive(Clone)]
pub struct CoclaiAgent {
    security_policy: AgentSecurityPolicy,
    state: Arc<Mutex<AgentState>>,
    state_store: Arc<dyn AgentStateStorePort + Send + Sync>,
    codex_gateway: Arc<dyn CodexGatewayPort + Send + Sync>,
}

impl Default for CoclaiAgent {
    fn default() -> Self {
        Self {
            security_policy: AgentSecurityPolicy::default(),
            state: Arc::new(Mutex::new(AgentState::default())),
            state_store: Arc::new(InMemoryAgentStateStore::new()),
            codex_gateway: Arc::new(TokioCodexGateway::new()),
        }
    }
}

impl CoclaiAgent {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_security_policy(security_policy: AgentSecurityPolicy) -> Self {
        Self {
            security_policy,
            state: Arc::new(Mutex::new(AgentState::default())),
            state_store: Arc::new(InMemoryAgentStateStore::new()),
            codex_gateway: Arc::new(TokioCodexGateway::new()),
        }
    }

    pub fn with_state_store(state_store: Arc<dyn AgentStateStorePort + Send + Sync>) -> Self {
        Self {
            state_store,
            ..Self::default()
        }
    }

    pub fn with_state_store_and_codex_gateway(
        state_store: Arc<dyn AgentStateStorePort + Send + Sync>,
        codex_gateway: Arc<dyn CodexGatewayPort + Send + Sync>,
    ) -> Self {
        Self {
            state_store,
            codex_gateway,
            ..Self::default()
        }
    }

    pub fn with_security_policy_and_state_store(
        security_policy: AgentSecurityPolicy,
        state_store: Arc<dyn AgentStateStorePort + Send + Sync>,
    ) -> Self {
        Self {
            security_policy,
            state: Arc::new(Mutex::new(AgentState::default())),
            state_store,
            codex_gateway: Arc::new(TokioCodexGateway::new()),
        }
    }

    pub fn with_security_policy_state_store_and_codex_gateway(
        security_policy: AgentSecurityPolicy,
        state_store: Arc<dyn AgentStateStorePort + Send + Sync>,
        codex_gateway: Arc<dyn CodexGatewayPort + Send + Sync>,
    ) -> Self {
        Self {
            security_policy,
            state: Arc::new(Mutex::new(AgentState::default())),
            state_store,
            codex_gateway,
        }
    }
}

#[cfg(test)]
mod tests;
