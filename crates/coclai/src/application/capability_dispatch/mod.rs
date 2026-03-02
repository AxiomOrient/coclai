use std::str::FromStr;
use std::time::Duration;

use serde_json::{json, Value};

use crate::api::{
    ApprovalPolicy, ExternalNetworkAccess, PromptAttachment, ReasoningEffort, SandboxPolicy,
    SandboxPreset,
};
use crate::client::RunProfile;
use crate::ergonomic::WorkflowConfig;

fn attachment_to_json(attachment: &PromptAttachment) -> Value {
    match attachment {
        PromptAttachment::AtPath { path, placeholder } => json!({
            "kind": "at_path",
            "path": path,
            "placeholder": placeholder,
        }),
        PromptAttachment::ImageUrl { url } => json!({
            "kind": "image_url",
            "url": url,
        }),
        PromptAttachment::LocalImage { path } => json!({
            "kind": "local_image",
            "path": path,
        }),
        PromptAttachment::Skill { name, path } => json!({
            "kind": "skill",
            "name": name,
            "path": path,
        }),
    }
}

fn parse_attachment(value: &Value) -> Result<PromptAttachment, String> {
    let obj = value
        .as_object()
        .ok_or_else(|| "attachment must be an object".to_owned())?;
    let kind = obj
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| "attachment.kind must be present".to_owned())?;

    match kind {
        "at_path" | "at-path" => {
            let path = obj
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| "at_path attachment requires path".to_owned())?;
            let placeholder = obj
                .get("placeholder")
                .and_then(Value::as_str)
                .map(|value| value.to_owned());
            Ok(PromptAttachment::AtPath {
                path: path.to_owned(),
                placeholder,
            })
        }
        "image_url" | "image-url" => {
            let url = obj
                .get("url")
                .and_then(Value::as_str)
                .ok_or_else(|| "image_url attachment requires url".to_owned())?;
            Ok(PromptAttachment::ImageUrl {
                url: url.to_owned(),
            })
        }
        "local_image" | "local-image" => {
            let path = obj
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| "local_image attachment requires path".to_owned())?;
            Ok(PromptAttachment::LocalImage {
                path: path.to_owned(),
            })
        }
        "skill" => {
            let name = obj
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| "skill attachment requires name".to_owned())?;
            let path = obj
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| "skill attachment requires path".to_owned())?;
            Ok(PromptAttachment::Skill {
                name: name.to_owned(),
                path: path.to_owned(),
            })
        }
        other => Err(format!("unsupported attachment kind `{other}`")),
    }
}

fn sandbox_policy_to_json(policy: &SandboxPolicy) -> Value {
    match policy {
        SandboxPolicy::Preset(SandboxPreset::ReadOnly) => json!("read_only"),
        SandboxPolicy::Preset(SandboxPreset::DangerFullAccess) => json!("danger_full_access"),
        SandboxPolicy::Preset(SandboxPreset::WorkspaceWrite {
            writable_roots,
            network_access,
        }) => json!({
            "preset": "workspace_write",
            "writable_roots": writable_roots,
            "network_access": network_access,
        }),
        SandboxPolicy::Preset(SandboxPreset::ExternalSandbox { network_access }) => json!({
            "raw": {
                "preset": "external_sandbox",
                "network_access": match network_access {
                    ExternalNetworkAccess::Restricted => "restricted",
                    ExternalNetworkAccess::Enabled => "enabled",
                }
            }
        }),
        SandboxPolicy::Raw(raw) => json!({
            "raw": raw,
        }),
    }
}

fn parse_sandbox_policy(value: &Value) -> Result<SandboxPolicy, String> {
    if let Some(raw_name) = value.as_str() {
        return match raw_name {
            "read_only" | "read-only" => Ok(SandboxPolicy::Preset(SandboxPreset::ReadOnly)),
            "danger_full_access" | "danger-full-access" => {
                Ok(SandboxPolicy::Preset(SandboxPreset::DangerFullAccess))
            }
            other => Err(format!("unsupported sandbox policy string `{other}`")),
        };
    }

    let obj = value
        .as_object()
        .ok_or_else(|| "sandbox_policy must be string or object".to_owned())?;
    if let Some(raw) = obj.get("raw") {
        return Ok(SandboxPolicy::Raw(raw.clone()));
    }

    let preset = obj
        .get("preset")
        .and_then(Value::as_str)
        .ok_or_else(|| "sandbox_policy.preset must be present".to_owned())?;

    match preset {
        "read_only" | "read-only" => Ok(SandboxPolicy::Preset(SandboxPreset::ReadOnly)),
        "danger_full_access" | "danger-full-access" => {
            Ok(SandboxPolicy::Preset(SandboxPreset::DangerFullAccess))
        }
        "workspace_write" | "workspace-write" => {
            let roots = obj
                .get("writable_roots")
                .and_then(Value::as_array)
                .ok_or_else(|| "workspace_write requires writable_roots array".to_owned())?;
            let mut writable_roots = Vec::with_capacity(roots.len());
            for root in roots {
                let root_value = root
                    .as_str()
                    .ok_or_else(|| "writable_roots values must be strings".to_owned())?;
                writable_roots.push(root_value.to_owned());
            }
            let network_access = obj
                .get("network_access")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            Ok(SandboxPolicy::Preset(SandboxPreset::WorkspaceWrite {
                writable_roots,
                network_access,
            }))
        }
        other => Err(format!("unsupported sandbox preset `{other}`")),
    }
}

fn run_profile_to_json(profile: &RunProfile) -> Value {
    json!({
        "model": profile.model,
        "effort": match profile.effort {
            ReasoningEffort::None => "none",
            ReasoningEffort::Minimal => "minimal",
            ReasoningEffort::Low => "low",
            ReasoningEffort::Medium => "medium",
            ReasoningEffort::High => "high",
            ReasoningEffort::XHigh => "xhigh",
        },
        "approval_policy": match profile.approval_policy {
            ApprovalPolicy::Untrusted => "untrusted",
            ApprovalPolicy::OnFailure => "on-failure",
            ApprovalPolicy::OnRequest => "on-request",
            ApprovalPolicy::Never => "never",
        },
        "timeout_ms": profile.timeout.as_millis() as u64,
        "allow_privileged_escalation": profile.privileged_escalation_approved,
        "sandbox_policy": sandbox_policy_to_json(&profile.sandbox_policy),
        "attachments": profile.attachments.iter().map(attachment_to_json).collect::<Vec<_>>(),
    })
}

fn parse_run_profile(profile_value: &Value) -> Result<RunProfile, String> {
    let profile_obj = profile_value
        .as_object()
        .ok_or_else(|| "profile must be an object".to_owned())?;

    let mut profile = RunProfile::new();
    if let Some(model) = profile_obj.get("model").and_then(Value::as_str) {
        profile = profile.with_model(model.to_owned());
    }
    if let Some(effort) = profile_obj.get("effort").and_then(Value::as_str) {
        let parsed = ReasoningEffort::from_str(effort)
            .map_err(|err| format!("invalid profile.effort `{effort}`: {err}"))?;
        profile = profile.with_effort(parsed);
    }
    if let Some(policy) = profile_obj.get("approval_policy").and_then(Value::as_str) {
        let parsed = ApprovalPolicy::from_str(policy)
            .map_err(|err| format!("invalid profile.approval_policy `{policy}`: {err}"))?;
        profile = profile.with_approval_policy(parsed);
    }
    if let Some(timeout_ms) = profile_obj.get("timeout_ms").and_then(Value::as_u64) {
        profile = profile.with_timeout(Duration::from_millis(timeout_ms));
    }
    if profile_obj
        .get("allow_privileged_escalation")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        profile = profile.allow_privileged_escalation();
    }
    if let Some(sandbox_value) = profile_obj.get("sandbox_policy") {
        profile = profile.with_sandbox_policy(parse_sandbox_policy(sandbox_value)?);
    }
    if let Some(attachments) = profile_obj.get("attachments") {
        let arr = attachments
            .as_array()
            .ok_or_else(|| "profile.attachments must be an array".to_owned())?;
        for attachment_value in arr {
            profile = profile.with_attachment(parse_attachment(attachment_value)?);
        }
    }
    Ok(profile)
}

pub fn encode_workflow_config(config: &WorkflowConfig) -> Value {
    json!({
        "cwd": config.cwd,
        "profile": run_profile_to_json(&config.run_profile),
    })
}

pub fn decode_workflow_config(raw: &Value) -> Result<WorkflowConfig, String> {
    let obj = raw
        .as_object()
        .ok_or_else(|| "workflow config store row is not an object".to_owned())?;
    let cwd = obj
        .get("cwd")
        .and_then(Value::as_str)
        .ok_or_else(|| "workflow config store row missing cwd".to_owned())?;

    let mut config = WorkflowConfig::new(cwd.to_owned());
    if let Some(profile_value) = obj.get("profile") {
        config = config.with_run_profile(parse_run_profile(profile_value)?);
    }
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::{decode_workflow_config, encode_workflow_config};
    use crate::api::{
        ApprovalPolicy, PromptAttachment, ReasoningEffort, SandboxPolicy, SandboxPreset,
    };
    use crate::client::RunProfile;
    use crate::ergonomic::WorkflowConfig;

    #[test]
    fn workflow_config_roundtrip_preserves_profile_fields() {
        let profile = RunProfile::new()
            .with_model("gpt-5-codex")
            .with_effort(ReasoningEffort::High)
            .with_approval_policy(ApprovalPolicy::OnRequest)
            .with_sandbox_policy(SandboxPolicy::Preset(SandboxPreset::WorkspaceWrite {
                writable_roots: vec!["/tmp".to_owned()],
                network_access: false,
            }))
            .attach_path("README.md")
            .with_attachment(PromptAttachment::ImageUrl {
                url: "https://example.com/diagram.png".to_owned(),
            });

        let original = WorkflowConfig::new("/tmp/workflow").with_run_profile(profile.clone());
        let encoded = encode_workflow_config(&original);
        let decoded = decode_workflow_config(&encoded).expect("workflow config should decode");

        assert_eq!(decoded.cwd, original.cwd);
        assert_eq!(decoded.run_profile.model, profile.model);
        assert_eq!(decoded.run_profile.effort, profile.effort);
        assert_eq!(decoded.run_profile.approval_policy, profile.approval_policy);
        assert_eq!(decoded.run_profile.sandbox_policy, profile.sandbox_policy);
        assert_eq!(decoded.run_profile.attachments, profile.attachments);
    }
}
