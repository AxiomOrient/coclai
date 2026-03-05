use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::str::FromStr;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ApprovalPolicy {
    #[serde(rename = "untrusted")]
    Untrusted,
    #[serde(rename = "on-failure")]
    OnFailure,
    #[serde(rename = "on-request")]
    OnRequest,
    #[serde(rename = "never")]
    Never,
}

impl ApprovalPolicy {
    pub fn as_wire(self) -> &'static str {
        match self {
            Self::Untrusted => "untrusted",
            Self::OnFailure => "on-failure",
            Self::OnRequest => "on-request",
            Self::Never => "never",
        }
    }
}

impl FromStr for ApprovalPolicy {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "untrusted" => Ok(Self::Untrusted),
            "on-failure" => Ok(Self::OnFailure),
            "on-request" => Ok(Self::OnRequest),
            "never" => Ok(Self::Never),
            other => Err(format!("unknown approval policy: {other}")),
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ReasoningEffort {
    #[serde(rename = "none")]
    None,
    #[serde(rename = "minimal")]
    Minimal,
    #[serde(rename = "low")]
    Low,
    #[serde(rename = "medium")]
    Medium,
    #[serde(rename = "high")]
    High,
    #[serde(rename = "xhigh")]
    XHigh,
}

impl ReasoningEffort {
    pub fn as_wire(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::XHigh => "xhigh",
        }
    }
}

pub const DEFAULT_REASONING_EFFORT: ReasoningEffort = ReasoningEffort::Medium;

impl FromStr for ReasoningEffort {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "none" => Ok(Self::None),
            "minimal" => Ok(Self::Minimal),
            "low" => Ok(Self::Low),
            "medium" => Ok(Self::Medium),
            "high" => Ok(Self::High),
            "xhigh" => Ok(Self::XHigh),
            other => Err(format!("unknown reasoning effort: {other}")),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExternalNetworkAccess {
    Restricted,
    Enabled,
}

impl ExternalNetworkAccess {
    pub fn as_wire(self) -> &'static str {
        match self {
            Self::Restricted => "restricted",
            Self::Enabled => "enabled",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SandboxPreset {
    ReadOnly,
    WorkspaceWrite {
        writable_roots: Vec<String>,
        network_access: bool,
    },
    DangerFullAccess,
    ExternalSandbox {
        network_access: ExternalNetworkAccess,
    },
}

impl SandboxPreset {
    pub fn as_type_wire(&self) -> &'static str {
        match self {
            Self::ReadOnly => "readOnly",
            Self::WorkspaceWrite { .. } => "workspaceWrite",
            Self::DangerFullAccess => "dangerFullAccess",
            Self::ExternalSandbox { .. } => "externalSandbox",
        }
    }

    pub fn as_legacy_wire(&self) -> Option<&'static str> {
        match self {
            Self::ReadOnly => Some("read-only"),
            Self::WorkspaceWrite { .. } => Some("workspace-write"),
            Self::DangerFullAccess => Some("danger-full-access"),
            Self::ExternalSandbox { .. } => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum SandboxPolicy {
    Preset(SandboxPreset),
    Raw(Value),
}
