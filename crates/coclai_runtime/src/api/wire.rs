use std::path::PathBuf;

use serde_json::{Map, Value};

use super::{
    ByteRange, ExternalNetworkAccess, InputItem, PromptAttachment, PromptRunError, SandboxPolicy,
    SandboxPreset, TextElement, ThreadStartParams, TurnStartParams,
};

pub(super) fn validate_prompt_attachments(
    cwd: &str,
    attachments: &[PromptAttachment],
) -> Result<(), PromptRunError> {
    for attachment in attachments {
        match attachment {
            PromptAttachment::AtPath { path, .. }
            | PromptAttachment::LocalImage { path }
            | PromptAttachment::Skill { path, .. } => {
                let resolved = resolve_attachment_path(cwd, path);
                if !resolved.exists() {
                    return Err(PromptRunError::AttachmentNotFound(
                        resolved.to_string_lossy().to_string(),
                    ));
                }
            }
            PromptAttachment::ImageUrl { .. } => {}
        }
    }
    Ok(())
}

fn resolve_attachment_path(cwd: &str, path: &str) -> PathBuf {
    let path = PathBuf::from(path);
    if path.is_absolute() {
        path
    } else {
        PathBuf::from(cwd).join(path)
    }
}

/// Map thread start parameters to wire JSON.
/// Allocation: one JSON object + selected optional fields.
/// Complexity: O(1) excluding nested JSON clone costs.
pub(super) fn thread_start_params_to_wire(p: &ThreadStartParams) -> Value {
    Value::Object(thread_overrides_to_wire(p))
}

/// Map thread override parameters to wire JSON.
/// Allocation: one JSON object + selected optional fields.
/// Complexity: O(1) excluding nested JSON clone costs.
pub(super) fn thread_overrides_to_wire(p: &ThreadStartParams) -> Map<String, Value> {
    let mut params = Map::<String, Value>::new();

    if let Some(model) = p.model.as_ref() {
        params.insert("model".to_owned(), Value::String(model.clone()));
    }
    if let Some(cwd) = p.cwd.as_ref() {
        params.insert("cwd".to_owned(), Value::String(cwd.clone()));
    }
    if let Some(approval_policy) = p.approval_policy.as_ref() {
        params.insert(
            "approvalPolicy".to_owned(),
            Value::String(approval_policy.as_wire().to_owned()),
        );
    }
    if let Some(sandbox_policy) = p.sandbox_policy.as_ref() {
        match sandbox_policy {
            SandboxPolicy::Preset(SandboxPreset::ReadOnly) => {
                params.insert("sandbox".to_owned(), Value::String("read-only".to_owned()));
            }
            SandboxPolicy::Preset(SandboxPreset::WorkspaceWrite { .. }) => {
                params.insert(
                    "sandbox".to_owned(),
                    Value::String("workspace-write".to_owned()),
                );
            }
            SandboxPolicy::Preset(SandboxPreset::DangerFullAccess) => {
                params.insert(
                    "sandbox".to_owned(),
                    Value::String("danger-full-access".to_owned()),
                );
            }
            other => {
                params.insert("sandboxPolicy".to_owned(), sandbox_policy_to_wire(other));
            }
        }
    }

    params
}

/// Map turn start parameters to wire JSON.
/// Allocation: one JSON object + input vector object allocations.
/// Complexity: O(n), n = input item count.
pub(super) fn turn_start_params_to_wire(thread_id: &str, p: &TurnStartParams) -> Value {
    let mut params = Map::<String, Value>::new();
    params.insert("threadId".to_owned(), Value::String(thread_id.to_owned()));
    params.insert(
        "input".to_owned(),
        Value::Array(p.input.iter().map(input_item_to_wire).collect()),
    );

    if let Some(cwd) = p.cwd.as_ref() {
        params.insert("cwd".to_owned(), Value::String(cwd.clone()));
    }
    if let Some(approval_policy) = p.approval_policy.as_ref() {
        params.insert(
            "approvalPolicy".to_owned(),
            Value::String(approval_policy.as_wire().to_owned()),
        );
    }
    if let Some(sandbox_policy) = p.sandbox_policy.as_ref() {
        params.insert(
            "sandboxPolicy".to_owned(),
            sandbox_policy_to_wire(sandbox_policy),
        );
    }
    if let Some(model) = p.model.as_ref() {
        params.insert("model".to_owned(), Value::String(model.clone()));
    }
    if let Some(effort) = p.effort.as_ref() {
        params.insert(
            "effort".to_owned(),
            Value::String(effort.as_wire().to_owned()),
        );
    }
    if let Some(summary) = p.summary.as_ref() {
        params.insert("summary".to_owned(), Value::String(summary.clone()));
    }
    if let Some(output_schema) = p.output_schema.as_ref() {
        params.insert("outputSchema".to_owned(), output_schema.clone());
    }

    Value::Object(params)
}

/// Build input items for one prompt execution.
/// Allocation: O(n), n = prompt length + attachment count.
pub(super) fn build_prompt_inputs(
    prompt: &str,
    attachments: &[PromptAttachment],
) -> Vec<InputItem> {
    let mut text = prompt.to_owned();
    let mut text_elements = Vec::<TextElement>::new();
    let mut tail_items = Vec::<InputItem>::new();

    for attachment in attachments {
        match attachment {
            PromptAttachment::AtPath { path, placeholder } => {
                append_at_path_mention(&mut text, &mut text_elements, path, placeholder.as_deref());
            }
            PromptAttachment::ImageUrl { url } => {
                tail_items.push(InputItem::ImageUrl { url: url.clone() });
            }
            PromptAttachment::LocalImage { path } => {
                tail_items.push(InputItem::LocalImage { path: path.clone() });
            }
            PromptAttachment::Skill { name, path } => {
                tail_items.push(InputItem::Skill {
                    name: name.clone(),
                    path: path.clone(),
                });
            }
        }
    }

    let mut input = Vec::<InputItem>::with_capacity(1 + tail_items.len());
    if text_elements.is_empty() {
        input.push(InputItem::Text { text });
    } else {
        input.push(InputItem::TextWithElements {
            text,
            text_elements,
        });
    }
    input.extend(tail_items);
    input
}

/// Append one @path mention and track its byte range.
/// Allocation: string growth for mention bytes + one text element.
/// Complexity: O(path length).
fn append_at_path_mention(
    text: &mut String,
    text_elements: &mut Vec<TextElement>,
    path: &str,
    placeholder: Option<&str>,
) {
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }

    let start = text.len() as u64;
    text.push('@');
    text.push_str(path);
    let end = text.len() as u64;

    text_elements.push(TextElement {
        byte_range: ByteRange { start, end },
        placeholder: placeholder.map(ToOwned::to_owned),
    });
}

/// Convert high-level input item enum to wire JSON.
/// Allocation: one JSON object per input item.
/// Complexity: O(1).
pub(super) fn input_item_to_wire(item: &InputItem) -> Value {
    let mut value = Map::<String, Value>::new();
    match item {
        InputItem::Text { text } => {
            value.insert("type".to_owned(), Value::String("text".to_owned()));
            value.insert("text".to_owned(), Value::String(text.clone()));
        }
        InputItem::TextWithElements {
            text,
            text_elements,
        } => {
            value.insert("type".to_owned(), Value::String("text".to_owned()));
            value.insert("text".to_owned(), Value::String(text.clone()));
            value.insert(
                "text_elements".to_owned(),
                Value::Array(text_elements.iter().map(text_element_to_wire).collect()),
            );
        }
        InputItem::ImageUrl { url } => {
            value.insert("type".to_owned(), Value::String("image".to_owned()));
            value.insert("url".to_owned(), Value::String(url.clone()));
        }
        InputItem::LocalImage { path } => {
            value.insert("type".to_owned(), Value::String("localImage".to_owned()));
            value.insert("path".to_owned(), Value::String(path.clone()));
        }
        InputItem::Skill { name, path } => {
            value.insert("type".to_owned(), Value::String("skill".to_owned()));
            value.insert("name".to_owned(), Value::String(name.clone()));
            value.insert("path".to_owned(), Value::String(path.clone()));
        }
    }
    Value::Object(value)
}

fn text_element_to_wire(element: &TextElement) -> Value {
    let mut obj = Map::<String, Value>::new();
    let mut byte_range = Map::<String, Value>::new();
    byte_range.insert(
        "start".to_owned(),
        Value::Number(serde_json::Number::from(element.byte_range.start)),
    );
    byte_range.insert(
        "end".to_owned(),
        Value::Number(serde_json::Number::from(element.byte_range.end)),
    );
    obj.insert("byteRange".to_owned(), Value::Object(byte_range));
    if let Some(placeholder) = element.placeholder.as_ref() {
        obj.insert("placeholder".to_owned(), Value::String(placeholder.clone()));
    }
    Value::Object(obj)
}

fn sandbox_policy_to_wire(policy: &SandboxPolicy) -> Value {
    match policy {
        SandboxPolicy::Preset(preset) => sandbox_preset_to_wire(preset),
        SandboxPolicy::Raw(value) => value.clone(),
    }
}

fn sandbox_preset_to_wire(preset: &SandboxPreset) -> Value {
    let mut value = Map::<String, Value>::new();
    match preset {
        SandboxPreset::ReadOnly => {
            value.insert("type".to_owned(), Value::String("readOnly".to_owned()));
        }
        SandboxPreset::WorkspaceWrite {
            writable_roots,
            network_access,
        } => {
            value.insert(
                "type".to_owned(),
                Value::String("workspaceWrite".to_owned()),
            );
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
        SandboxPreset::DangerFullAccess => {
            value.insert(
                "type".to_owned(),
                Value::String("dangerFullAccess".to_owned()),
            );
        }
        SandboxPreset::ExternalSandbox { network_access } => {
            value.insert(
                "type".to_owned(),
                Value::String("externalSandbox".to_owned()),
            );
            value.insert(
                "networkAccess".to_owned(),
                Value::String(
                    match network_access {
                        ExternalNetworkAccess::Restricted => "restricted",
                        ExternalNetworkAccess::Enabled => "enabled",
                    }
                    .to_owned(),
                ),
            );
        }
    }
    Value::Object(value)
}
