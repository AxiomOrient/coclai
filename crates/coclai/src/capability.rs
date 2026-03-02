use std::fmt::Write;
use std::str::FromStr;

use crate::appserver::methods;
use serde::{Deserialize, Serialize};

/// External ingress targets for `coclai-agent`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityIngress {
    Stdio,
    HttpLocalhost,
    WebSocketLocalhost,
}

impl CapabilityIngress {
    pub const ALL: [Self; 3] = [Self::Stdio, Self::HttpLocalhost, Self::WebSocketLocalhost];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Stdio => "stdio",
            Self::HttpLocalhost => "http(localhost)",
            Self::WebSocketLocalhost => "ws(localhost)",
        }
    }
}

impl FromStr for CapabilityIngress {
    type Err = &'static str;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "stdio" => Ok(Self::Stdio),
            "http" | "http-localhost" | "http(localhost)" => Ok(Self::HttpLocalhost),
            "ws" | "websocket" | "ws-localhost" | "ws(localhost)" => Ok(Self::WebSocketLocalhost),
            _ => Err("invalid ingress (allowed: stdio|http|ws)"),
        }
    }
}

/// Exposure status per ingress.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CapabilityExposure {
    Available,
    Planned,
    NotApplicable,
}

impl CapabilityExposure {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Available => "available",
            Self::Planned => "planned",
            Self::NotApplicable => "n/a",
        }
    }
}

/// Ingress exposure map for one capability.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CapabilityIngressSupport {
    pub stdio: CapabilityExposure,
    pub http_localhost: CapabilityExposure,
    pub ws_localhost: CapabilityExposure,
}

impl CapabilityIngressSupport {
    pub const fn get(self, ingress: CapabilityIngress) -> CapabilityExposure {
        match ingress {
            CapabilityIngress::Stdio => self.stdio,
            CapabilityIngress::HttpLocalhost => self.http_localhost,
            CapabilityIngress::WebSocketLocalhost => self.ws_localhost,
        }
    }

    pub const fn is_full_parity_available(self) -> bool {
        matches!(self.stdio, CapabilityExposure::Available)
            && matches!(self.http_localhost, CapabilityExposure::Available)
            && matches!(self.ws_localhost, CapabilityExposure::Available)
    }
}

/// Capability descriptor for external contracts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CapabilityDescriptor {
    pub capability_id: &'static str,
    pub surface: &'static str,
    pub summary: &'static str,
    pub ingress: CapabilityIngressSupport,
}

const ALL_INGRESS_AVAILABLE: CapabilityIngressSupport = CapabilityIngressSupport {
    stdio: CapabilityExposure::Available,
    http_localhost: CapabilityExposure::Available,
    ws_localhost: CapabilityExposure::Available,
};

const CAPABILITY_REGISTRY: &[CapabilityDescriptor] = &[
    CapabilityDescriptor {
        capability_id: "system/health",
        surface: "coclai::CoclaiAgent::health/dispatch",
        summary: "agent health and registry snapshot",
        ingress: ALL_INGRESS_AVAILABLE,
    },
    CapabilityDescriptor {
        capability_id: "system/capability_registry",
        surface: "coclai::CoclaiAgent::dispatch",
        summary: "list capability registry rows",
        ingress: ALL_INGRESS_AVAILABLE,
    },
    CapabilityDescriptor {
        capability_id: "system/capability_parity_report",
        surface: "coclai::CoclaiAgent::dispatch",
        summary: "render parity report markdown",
        ingress: ALL_INGRESS_AVAILABLE,
    },
    CapabilityDescriptor {
        capability_id: "quick_run",
        surface: "coclai::quick_run",
        summary: "one-shot prompt run with default profile",
        ingress: ALL_INGRESS_AVAILABLE,
    },
    CapabilityDescriptor {
        capability_id: "quick_run_with_profile",
        surface: "coclai::quick_run_with_profile",
        summary: "one-shot prompt run with explicit profile",
        ingress: ALL_INGRESS_AVAILABLE,
    },
    CapabilityDescriptor {
        capability_id: "workflow/connect",
        surface: "coclai::Workflow::connect",
        summary: "connect reusable workflow handle",
        ingress: ALL_INGRESS_AVAILABLE,
    },
    CapabilityDescriptor {
        capability_id: "workflow/run",
        surface: "coclai::Workflow::run",
        summary: "run prompt via reusable workflow session defaults",
        ingress: ALL_INGRESS_AVAILABLE,
    },
    CapabilityDescriptor {
        capability_id: "workflow/session/setup",
        surface: "coclai::Workflow::setup_session",
        summary: "setup one explicit session for multi-turn control",
        ingress: ALL_INGRESS_AVAILABLE,
    },
    CapabilityDescriptor {
        capability_id: "appserver/request/json",
        surface: "coclai::AppServer::request_json",
        summary: "validated JSON-RPC request",
        ingress: ALL_INGRESS_AVAILABLE,
    },
    CapabilityDescriptor {
        capability_id: "appserver/request/typed",
        surface: "coclai::AppServer::request_typed",
        summary: "typed JSON-RPC request",
        ingress: ALL_INGRESS_AVAILABLE,
    },
    CapabilityDescriptor {
        capability_id: "appserver/notify/json",
        surface: "coclai::AppServer::notify_json",
        summary: "validated JSON-RPC notification",
        ingress: ALL_INGRESS_AVAILABLE,
    },
    CapabilityDescriptor {
        capability_id: "appserver/server-requests/take",
        surface: "coclai::AppServer::take_server_requests",
        summary: "consume server request stream (approval/user-input/tool)",
        ingress: ALL_INGRESS_AVAILABLE,
    },
    CapabilityDescriptor {
        capability_id: "appserver/server-requests/respond/ok",
        surface: "coclai::AppServer::respond_server_request_ok",
        summary: "send server-request success response",
        ingress: ALL_INGRESS_AVAILABLE,
    },
    CapabilityDescriptor {
        capability_id: "appserver/server-requests/respond/err",
        surface: "coclai::AppServer::respond_server_request_err",
        summary: "send server-request error response",
        ingress: ALL_INGRESS_AVAILABLE,
    },
    CapabilityDescriptor {
        capability_id: methods::THREAD_START,
        surface: "rpc.thread.start",
        summary: "start thread",
        ingress: ALL_INGRESS_AVAILABLE,
    },
    CapabilityDescriptor {
        capability_id: methods::THREAD_RESUME,
        surface: "rpc.thread.resume",
        summary: "resume thread",
        ingress: ALL_INGRESS_AVAILABLE,
    },
    CapabilityDescriptor {
        capability_id: methods::THREAD_FORK,
        surface: "rpc.thread.fork",
        summary: "fork thread",
        ingress: ALL_INGRESS_AVAILABLE,
    },
    CapabilityDescriptor {
        capability_id: methods::THREAD_ARCHIVE,
        surface: "rpc.thread.archive",
        summary: "archive thread",
        ingress: ALL_INGRESS_AVAILABLE,
    },
    CapabilityDescriptor {
        capability_id: methods::THREAD_READ,
        surface: "rpc.thread.read",
        summary: "read thread detail",
        ingress: ALL_INGRESS_AVAILABLE,
    },
    CapabilityDescriptor {
        capability_id: methods::THREAD_LIST,
        surface: "rpc.thread.list",
        summary: "list thread summaries",
        ingress: ALL_INGRESS_AVAILABLE,
    },
    CapabilityDescriptor {
        capability_id: methods::THREAD_LOADED_LIST,
        surface: "rpc.thread.loaded.list",
        summary: "list loaded threads",
        ingress: ALL_INGRESS_AVAILABLE,
    },
    CapabilityDescriptor {
        capability_id: methods::THREAD_ROLLBACK,
        surface: "rpc.thread.rollback",
        summary: "rollback thread to one turn",
        ingress: ALL_INGRESS_AVAILABLE,
    },
    CapabilityDescriptor {
        capability_id: methods::TURN_START,
        surface: "rpc.turn.start",
        summary: "start turn in one thread",
        ingress: ALL_INGRESS_AVAILABLE,
    },
    CapabilityDescriptor {
        capability_id: methods::TURN_INTERRUPT,
        surface: "rpc.turn.interrupt",
        summary: "interrupt in-flight turn",
        ingress: ALL_INGRESS_AVAILABLE,
    },
];

/// Full capability registry used as SoT for external exposure planning.
pub fn capability_registry() -> &'static [CapabilityDescriptor] {
    CAPABILITY_REGISTRY
}

/// Lookup one capability by id.
pub fn capability_by_id(capability_id: &str) -> Option<&'static CapabilityDescriptor> {
    CAPABILITY_REGISTRY
        .iter()
        .find(|descriptor| descriptor.capability_id == capability_id)
}

/// List capabilities that are not marked `available` for one ingress.
pub fn missing_capabilities_for_ingress(
    ingress: CapabilityIngress,
) -> Vec<&'static CapabilityDescriptor> {
    CAPABILITY_REGISTRY
        .iter()
        .filter(|descriptor| descriptor.ingress.get(ingress) != CapabilityExposure::Available)
        .collect()
}

/// List capabilities that are not yet full parity available on every ingress.
pub fn capability_parity_gaps() -> Vec<&'static CapabilityDescriptor> {
    CAPABILITY_REGISTRY
        .iter()
        .filter(|descriptor| !descriptor.ingress.is_full_parity_available())
        .collect()
}

/// Render a deterministic markdown report for parity tracking.
pub fn render_capability_parity_report() -> String {
    let mut report = String::new();
    writeln!(
        report,
        "# Capability Parity Report\n\n- registry_size: {}\n- full_parity_gaps: {}\n",
        CAPABILITY_REGISTRY.len(),
        capability_parity_gaps().len()
    )
    .ok();
    writeln!(
        report,
        "## Capability Rows\n\nFormat: `capability_id | surface | stdio | http(localhost) | ws(localhost) | summary`\n"
    )
    .ok();
    for descriptor in CAPABILITY_REGISTRY {
        writeln!(
            report,
            "- capability_id: {} | {} | {} | {} | {} | {}",
            descriptor.capability_id,
            descriptor.surface,
            descriptor.ingress.stdio.as_str(),
            descriptor.ingress.http_localhost.as_str(),
            descriptor.ingress.ws_localhost.as_str(),
            descriptor.summary
        )
        .ok();
    }

    report
}

#[cfg(test)]
mod tests;
