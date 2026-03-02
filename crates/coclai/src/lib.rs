#![allow(dead_code)]

//! Public facade for coclai.
//! This crate is intentionally monolithic: runtime/plugin/facade are compiled here.

mod plugin_core_contract;

mod api;
mod approvals;
mod client;
mod errors;
mod events;
mod hooks;
mod metrics;
mod rpc;
mod rpc_contract;
mod runtime;
pub(crate) mod runtime_schema;
mod schema;
mod sink;
mod state;
mod transport;
mod turn_output;

pub(crate) mod adapters;
mod agent;
pub(crate) mod application;
mod appserver;
pub(crate) mod bootstrap;
mod capability;
pub(crate) mod domain;
mod ergonomic;
pub(crate) mod ports;

pub use bootstrap::container::build_agent;
pub(crate) type ServerRequestRx = tokio::sync::mpsc::Receiver<approvals::ServerRequest>;

pub use agent::{
    AgentDispatchError, AgentHealth, AgentSecurityPolicy, CapabilityInvocation, CapabilityResponse,
    CoclaiAgent,
};
pub use capability::{
    capability_by_id, capability_parity_gaps, capability_registry,
    missing_capabilities_for_ingress, render_capability_parity_report, CapabilityDescriptor,
    CapabilityExposure, CapabilityIngress, CapabilityIngressSupport,
};
