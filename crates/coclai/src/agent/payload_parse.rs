use std::str::FromStr;
use std::time::Duration;

use serde_json::{Map, Value};

use crate::agent::{AgentDispatchError, CapabilityInvocation};
use crate::api::{ApprovalPolicy, PromptAttachment, ReasoningEffort, SandboxPolicy, SandboxPreset};
use crate::client::RunProfile;

pub(super) fn parse_run_profile(profile_value: &Value) -> Result<RunProfile, String> {
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

    if let Some(allow) = profile_obj
        .get("allow_privileged_escalation")
        .and_then(Value::as_bool)
    {
        if allow {
            profile = profile.allow_privileged_escalation();
        }
    }

    if let Some(sandbox_value) = profile_obj.get("sandbox_policy") {
        let parsed = parse_sandbox_policy(sandbox_value)?;
        profile = profile.with_sandbox_policy(parsed);
    }

    if let Some(attachments) = profile_obj.get("attachments") {
        let arr = attachments
            .as_array()
            .ok_or_else(|| "profile.attachments must be an array".to_owned())?;
        for attachment_value in arr {
            let attachment = parse_attachment(attachment_value)?;
            profile = profile.with_attachment(attachment);
        }
    }

    Ok(profile)
}

pub(super) fn parse_sandbox_policy(value: &Value) -> Result<SandboxPolicy, String> {
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

pub(super) fn parse_attachment(value: &Value) -> Result<PromptAttachment, String> {
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

pub(super) fn payload_as_object(
    invocation: &CapabilityInvocation,
) -> Result<&Map<String, Value>, AgentDispatchError> {
    invocation
        .payload
        .as_object()
        .ok_or_else(|| AgentDispatchError::InvalidPayload {
            capability_id: invocation.capability_id.clone(),
            message: "payload must be a JSON object".to_owned(),
        })
}

pub(super) fn require_string_field(
    capability_id: &str,
    obj: &Map<String, Value>,
    key: &str,
) -> Result<String, AgentDispatchError> {
    obj.get(key)
        .and_then(Value::as_str)
        .map(|value| value.to_owned())
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| AgentDispatchError::InvalidPayload {
            capability_id: capability_id.to_owned(),
            message: format!("payload.{key} must be a non-empty string"),
        })
}

pub(super) fn optional_string_field(obj: &Map<String, Value>, key: &str) -> Option<String> {
    obj.get(key)
        .and_then(Value::as_str)
        .map(|value| value.to_owned())
        .filter(|value| !value.trim().is_empty())
}

pub(super) fn maybe_profile_field(
    capability_id: &str,
    obj: &Map<String, Value>,
    key: &str,
) -> Result<Option<RunProfile>, AgentDispatchError> {
    let Some(profile_value) = obj.get(key) else {
        return Ok(None);
    };
    parse_run_profile(profile_value)
        .map(Some)
        .map_err(|message| AgentDispatchError::InvalidPayload {
            capability_id: capability_id.to_owned(),
            message,
        })
}
