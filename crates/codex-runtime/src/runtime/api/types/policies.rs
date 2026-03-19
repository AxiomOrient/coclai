use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::str::FromStr;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ServiceTier {
    #[serde(rename = "fast")]
    Fast,
    #[serde(rename = "flex")]
    Flex,
}

impl ServiceTier {
    pub fn as_wire(self) -> &'static str {
        match self {
            Self::Fast => "fast",
            Self::Flex => "flex",
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Personality {
    #[serde(rename = "none")]
    None,
    #[serde(rename = "friendly")]
    Friendly,
    #[serde(rename = "pragmatic")]
    Pragmatic,
}

impl Personality {
    pub fn as_wire(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Friendly => "friendly",
            Self::Pragmatic => "pragmatic",
        }
    }
}

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
            Self::ReadOnly => SANDBOX_POLICY_TYPE_READ_ONLY,
            Self::WorkspaceWrite { .. } => SANDBOX_POLICY_TYPE_WORKSPACE_WRITE,
            Self::DangerFullAccess => SANDBOX_POLICY_TYPE_DANGER_FULL_ACCESS,
            Self::ExternalSandbox { .. } => SANDBOX_POLICY_TYPE_EXTERNAL_SANDBOX,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum SandboxPolicy {
    Preset(SandboxPreset),
    Raw(Value),
}

const SANDBOX_POLICY_TYPE_READ_ONLY: &str = "readOnly";
const SANDBOX_POLICY_TYPE_WORKSPACE_WRITE: &str = "workspaceWrite";
const SANDBOX_POLICY_TYPE_DANGER_FULL_ACCESS: &str = "dangerFullAccess";
const SANDBOX_POLICY_TYPE_EXTERNAL_SANDBOX: &str = "externalSandbox";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SandboxPolicyKind {
    ReadOnly,
    WorkspaceWrite,
    DangerFullAccess,
    ExternalSandbox,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SandboxPolicySummary {
    kind: SandboxPolicyKind,
    has_non_empty_writable_roots: bool,
}

impl SandboxPolicySummary {
    pub(crate) fn is_privileged(self) -> bool {
        !matches!(self.kind, SandboxPolicyKind::ReadOnly)
    }

    pub(crate) fn has_non_empty_writable_roots(self) -> bool {
        self.has_non_empty_writable_roots
    }
}

pub(crate) fn summarize_sandbox_policy(
    policy: &SandboxPolicy,
) -> Result<SandboxPolicySummary, String> {
    match policy {
        SandboxPolicy::Preset(preset) => Ok(summarize_sandbox_preset(preset)),
        SandboxPolicy::Raw(value) => summarize_sandbox_policy_wire_value(value, "sandboxPolicy"),
    }
}

pub(crate) fn summarize_sandbox_policy_wire_value(
    value: &Value,
    field_path: &str,
) -> Result<SandboxPolicySummary, String> {
    let policy_obj = value
        .as_object()
        .ok_or_else(|| format!("{field_path} must be an object when provided"))?;
    let policy_type = parse_sandbox_policy_type(policy_obj, field_path)?;
    Ok(SandboxPolicySummary {
        kind: sandbox_policy_kind_from_wire(policy_type),
        has_non_empty_writable_roots: writable_roots_non_empty(policy_obj),
    })
}

pub(crate) fn sandbox_policy_to_wire_value(policy: &SandboxPolicy) -> Value {
    match policy {
        SandboxPolicy::Preset(preset) => sandbox_preset_to_wire_value(preset),
        SandboxPolicy::Raw(value) => value.clone(),
    }
}

fn summarize_sandbox_preset(preset: &SandboxPreset) -> SandboxPolicySummary {
    match preset {
        SandboxPreset::ReadOnly => SandboxPolicySummary {
            kind: SandboxPolicyKind::ReadOnly,
            has_non_empty_writable_roots: false,
        },
        SandboxPreset::WorkspaceWrite { writable_roots, .. } => SandboxPolicySummary {
            kind: SandboxPolicyKind::WorkspaceWrite,
            has_non_empty_writable_roots: writable_roots.iter().any(|root| !root.trim().is_empty()),
        },
        SandboxPreset::DangerFullAccess => SandboxPolicySummary {
            kind: SandboxPolicyKind::DangerFullAccess,
            has_non_empty_writable_roots: false,
        },
        SandboxPreset::ExternalSandbox { .. } => SandboxPolicySummary {
            kind: SandboxPolicyKind::ExternalSandbox,
            has_non_empty_writable_roots: false,
        },
    }
}

fn parse_sandbox_policy_type<'a>(
    policy_obj: &'a Map<String, Value>,
    field_path: &str,
) -> Result<&'a str, String> {
    let policy_type = policy_obj
        .get("type")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{field_path}.type must be a non-empty string"))?;
    Ok(policy_type)
}

fn sandbox_policy_kind_from_wire(policy_type: &str) -> SandboxPolicyKind {
    match policy_type {
        SANDBOX_POLICY_TYPE_READ_ONLY => SandboxPolicyKind::ReadOnly,
        SANDBOX_POLICY_TYPE_WORKSPACE_WRITE => SandboxPolicyKind::WorkspaceWrite,
        SANDBOX_POLICY_TYPE_DANGER_FULL_ACCESS => SandboxPolicyKind::DangerFullAccess,
        SANDBOX_POLICY_TYPE_EXTERNAL_SANDBOX => SandboxPolicyKind::ExternalSandbox,
        _ => SandboxPolicyKind::Unknown,
    }
}

fn writable_roots_non_empty(policy_obj: &Map<String, Value>) -> bool {
    let Some(roots) = policy_obj.get("writableRoots").and_then(Value::as_array) else {
        return false;
    };
    roots
        .iter()
        .filter_map(Value::as_str)
        .any(|root| !root.trim().is_empty())
}

fn sandbox_preset_to_wire_value(preset: &SandboxPreset) -> Value {
    let mut value = Map::<String, Value>::new();
    value.insert(
        "type".to_owned(),
        Value::String(preset.as_type_wire().to_owned()),
    );
    match preset {
        SandboxPreset::ReadOnly | SandboxPreset::DangerFullAccess => {}
        SandboxPreset::WorkspaceWrite {
            writable_roots,
            network_access,
        } => {
            value.insert(
                "writableRoots".to_owned(),
                Value::Array(
                    writable_roots
                        .iter()
                        .map(|root| Value::String(root.clone()))
                        .collect(),
                ),
            );
            value.insert("networkAccess".to_owned(), Value::Bool(*network_access));
        }
        SandboxPreset::ExternalSandbox { network_access } => {
            value.insert(
                "networkAccess".to_owned(),
                Value::String(network_access.as_wire().to_owned()),
            );
        }
    }
    Value::Object(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn summarize_sandbox_policy_read_only_is_not_privileged() {
        let summary = summarize_sandbox_policy(&SandboxPolicy::Preset(SandboxPreset::ReadOnly))
            .expect("preset summary");
        assert!(!summary.is_privileged());
        assert!(!summary.has_non_empty_writable_roots());
    }

    #[test]
    fn summarize_sandbox_policy_workspace_write_tracks_non_empty_roots() {
        let summary =
            summarize_sandbox_policy(&SandboxPolicy::Preset(SandboxPreset::WorkspaceWrite {
                writable_roots: vec!["".to_owned(), " /tmp ".to_owned()],
                network_access: false,
            }))
            .expect("preset summary");
        assert!(summary.is_privileged());
        assert!(summary.has_non_empty_writable_roots());
    }

    #[test]
    fn summarize_sandbox_policy_wire_requires_object() {
        let err = summarize_sandbox_policy_wire_value(&json!("read-only"), "params.sandboxPolicy")
            .expect_err("non-object must fail");
        assert_eq!(err, "params.sandboxPolicy must be an object when provided");
    }

    #[test]
    fn summarize_sandbox_policy_wire_requires_non_empty_type() {
        let err = summarize_sandbox_policy_wire_value(
            &json!({"type":"   ", "writableRoots":["/tmp"]}),
            "params.sandboxPolicy",
        )
        .expect_err("empty type must fail");
        assert_eq!(err, "params.sandboxPolicy.type must be a non-empty string");
    }

    #[test]
    fn sandbox_policy_to_wire_value_emits_workspace_write_shape() {
        let wire =
            sandbox_policy_to_wire_value(&SandboxPolicy::Preset(SandboxPreset::WorkspaceWrite {
                writable_roots: vec!["/tmp".to_owned()],
                network_access: false,
            }));
        assert_eq!(wire["type"], "workspaceWrite");
        assert_eq!(wire["writableRoots"][0], "/tmp");
        assert_eq!(wire["networkAccess"], false);
    }

    #[test]
    fn summarize_sandbox_policy_wire_accepts_legacy_camel_case_aliases() {
        let read_only = summarize_sandbox_policy_wire_value(
            &json!({"type":"readOnly"}),
            "params.sandboxPolicy",
        )
        .expect("legacy readOnly must still classify");
        assert!(!read_only.is_privileged());

        let workspace_write = summarize_sandbox_policy_wire_value(
            &json!({"type":"workspaceWrite","writableRoots":["/tmp"]}),
            "params.sandboxPolicy",
        )
        .expect("legacy workspaceWrite must still classify");
        assert!(workspace_write.is_privileged());
        assert!(workspace_write.has_non_empty_writable_roots());
    }
}
