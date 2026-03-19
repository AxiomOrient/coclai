use serde::{de::DeserializeOwned, Serialize};
use serde_json::{Map, Value};

use crate::runtime::errors::RpcError;
use crate::runtime::rpc_contract::payload_summary;

use super::{
    sandbox_policy_to_wire_value, summarize_sandbox_policy, ApprovalPolicy, ByteRange,
    CommandExecParams, InputItem, PromptAttachment, TextElement, ThreadStartParams,
    TurnStartParams,
};

pub(super) fn serialize_params<T: Serialize>(method: &str, params: &T) -> Result<Value, RpcError> {
    serde_json::to_value(params)
        .map_err(|error| RpcError::InvalidRequest(format!("{method} invalid params: {error}")))
}

pub(super) fn deserialize_result<T: DeserializeOwned>(
    method: &str,
    response: Value,
) -> Result<T, RpcError> {
    let response_summary = payload_summary(&response);
    serde_json::from_value(response).map_err(|error| {
        RpcError::InvalidRequest(format!(
            "{method} invalid result: {error}; response: {response_summary}"
        ))
    })
}

/// Enforce privileged sandbox escalation policy (SEC-004) for session-start/resume.
/// High-risk sandbox usage requires:
/// 1) explicit opt-in (`privileged_escalation_approved`)
/// 2) non-never approval policy
/// 3) explicit execution scope (`cwd` or writable roots)
pub(super) fn validate_thread_start_security(p: &ThreadStartParams) -> Result<(), RpcError> {
    let Some(sandbox_policy) = p.sandbox_policy.as_ref() else {
        return Ok(());
    };
    let policy_summary =
        summarize_sandbox_policy(sandbox_policy).map_err(RpcError::InvalidRequest)?;
    if !policy_summary.is_privileged() {
        return Ok(());
    }
    if !p.privileged_escalation_approved {
        return Err(RpcError::InvalidRequest(
            "privileged sandbox requires explicit escalation approval".to_owned(),
        ));
    }
    let approval = p.approval_policy.unwrap_or(ApprovalPolicy::Never);
    if approval == ApprovalPolicy::Never {
        return Err(RpcError::InvalidRequest(
            "privileged sandbox requires non-never approval policy".to_owned(),
        ));
    }
    if !has_explicit_scope(
        p.cwd.as_deref(),
        policy_summary.has_non_empty_writable_roots(),
    ) {
        return Err(RpcError::InvalidRequest(
            "privileged sandbox requires explicit scope via cwd or writable roots".to_owned(),
        ));
    }
    Ok(())
}

/// Enforce privileged sandbox escalation policy (SEC-004) for turn/start.
pub(super) fn validate_turn_start_security(p: &TurnStartParams) -> Result<(), RpcError> {
    let Some(sandbox_policy) = p.sandbox_policy.as_ref() else {
        return Ok(());
    };
    let policy_summary =
        summarize_sandbox_policy(sandbox_policy).map_err(RpcError::InvalidRequest)?;
    if !policy_summary.is_privileged() {
        return Ok(());
    }
    if !p.privileged_escalation_approved {
        return Err(RpcError::InvalidRequest(
            "privileged sandbox requires explicit escalation approval".to_owned(),
        ));
    }
    let approval = p.approval_policy.unwrap_or(ApprovalPolicy::Never);
    if approval == ApprovalPolicy::Never {
        return Err(RpcError::InvalidRequest(
            "privileged sandbox requires non-never approval policy".to_owned(),
        ));
    }
    if !has_explicit_scope(
        p.cwd.as_deref(),
        policy_summary.has_non_empty_writable_roots(),
    ) {
        return Err(RpcError::InvalidRequest(
            "privileged sandbox requires explicit scope via cwd or writable roots".to_owned(),
        ));
    }
    Ok(())
}

fn has_explicit_scope(cwd: Option<&str>, has_non_empty_writable_roots: bool) -> bool {
    if cwd.is_some_and(|v| !v.trim().is_empty()) {
        return true;
    }
    has_non_empty_writable_roots
}

/// Map thread start parameters to wire JSON.
/// Allocation: one JSON object + selected optional fields.
/// Complexity: O(1) excluding nested JSON clone costs.
pub(super) fn thread_start_params_to_wire(p: &ThreadStartParams) -> Value {
    let mut params = Map::<String, Value>::new();
    insert_thread_common_overrides(&mut params, p);

    if let Some(service_name) = p.service_name.as_ref() {
        params.insert(
            "serviceName".to_owned(),
            Value::String(service_name.clone()),
        );
    }
    if let Some(ephemeral) = p.ephemeral {
        params.insert("ephemeral".to_owned(), Value::Bool(ephemeral));
    }

    Value::Object(params)
}

/// Map thread override parameters to wire JSON.
/// Allocation: one JSON object + selected optional fields.
/// Complexity: O(1) excluding nested JSON clone costs.
pub(super) fn thread_overrides_to_wire(p: &ThreadStartParams) -> Map<String, Value> {
    let mut params = Map::<String, Value>::new();
    insert_thread_common_overrides(&mut params, p);
    params
}

fn insert_thread_common_overrides(params: &mut Map<String, Value>, p: &ThreadStartParams) {
    if let Some(model) = p.model.as_ref() {
        params.insert("model".to_owned(), Value::String(model.clone()));
    }
    if let Some(model_provider) = p.model_provider.as_ref() {
        params.insert(
            "modelProvider".to_owned(),
            Value::String(model_provider.clone()),
        );
    }
    if let Some(service_tier) = p.service_tier {
        params.insert(
            "serviceTier".to_owned(),
            match service_tier {
                Some(service_tier) => Value::String(service_tier.as_wire().to_owned()),
                None => Value::Null,
            },
        );
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
    insert_privileged_escalation_approved(params, p.privileged_escalation_approved);
    if let Some(sandbox_policy) = p.sandbox_policy.as_ref() {
        params.insert(
            "sandboxPolicy".to_owned(),
            sandbox_policy_to_wire_value(sandbox_policy),
        );
    }
    if let Some(config) = p.config.as_ref() {
        params.insert("config".to_owned(), Value::Object(config.clone()));
    }
    if let Some(base_instructions) = p.base_instructions.as_ref() {
        params.insert(
            "baseInstructions".to_owned(),
            Value::String(base_instructions.clone()),
        );
    }
    if let Some(developer_instructions) = p.developer_instructions.as_ref() {
        params.insert(
            "developerInstructions".to_owned(),
            Value::String(developer_instructions.clone()),
        );
    }
    if let Some(personality) = p.personality {
        params.insert(
            "personality".to_owned(),
            Value::String(personality.as_wire().to_owned()),
        );
    }
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
    insert_privileged_escalation_approved(&mut params, p.privileged_escalation_approved);
    if let Some(sandbox_policy) = p.sandbox_policy.as_ref() {
        params.insert(
            "sandboxPolicy".to_owned(),
            sandbox_policy_to_wire_value(sandbox_policy),
        );
    }
    if let Some(model) = p.model.as_ref() {
        params.insert("model".to_owned(), Value::String(model.clone()));
    }
    if let Some(service_tier) = p.service_tier {
        params.insert(
            "serviceTier".to_owned(),
            match service_tier {
                Some(service_tier) => Value::String(service_tier.as_wire().to_owned()),
                None => Value::Null,
            },
        );
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
    if let Some(personality) = p.personality {
        params.insert(
            "personality".to_owned(),
            Value::String(personality.as_wire().to_owned()),
        );
    }
    if let Some(output_schema) = p.output_schema.as_ref() {
        params.insert("outputSchema".to_owned(), output_schema.clone());
    }

    Value::Object(params)
}

/// Map command/exec parameters to wire JSON.
/// Allocation: one JSON object plus optional nested env/sandbox objects.
/// Complexity: O(n), n = env entry count + command argv length.
pub(super) fn command_exec_params_to_wire(p: &CommandExecParams) -> Value {
    let mut params = Map::<String, Value>::new();
    params.insert(
        "command".to_owned(),
        Value::Array(
            p.command
                .iter()
                .map(|part| Value::String(part.clone()))
                .collect(),
        ),
    );

    if let Some(process_id) = p.process_id.as_ref() {
        params.insert("processId".to_owned(), Value::String(process_id.clone()));
    }
    if p.tty {
        params.insert("tty".to_owned(), Value::Bool(true));
        params.insert("streamStdin".to_owned(), Value::Bool(true));
        params.insert("streamStdoutStderr".to_owned(), Value::Bool(true));
    } else {
        if p.stream_stdin {
            params.insert("streamStdin".to_owned(), Value::Bool(true));
        }
        if p.stream_stdout_stderr {
            params.insert("streamStdoutStderr".to_owned(), Value::Bool(true));
        }
    }
    if let Some(output_bytes_cap) = p.output_bytes_cap {
        params.insert(
            "outputBytesCap".to_owned(),
            Value::Number(serde_json::Number::from(output_bytes_cap as u64)),
        );
    }
    if p.disable_output_cap {
        params.insert("disableOutputCap".to_owned(), Value::Bool(true));
    }
    if p.disable_timeout {
        params.insert("disableTimeout".to_owned(), Value::Bool(true));
    }
    if let Some(timeout_ms) = p.timeout_ms {
        params.insert(
            "timeoutMs".to_owned(),
            Value::Number(serde_json::Number::from(timeout_ms)),
        );
    }
    if let Some(cwd) = p.cwd.as_ref() {
        params.insert("cwd".to_owned(), Value::String(cwd.clone()));
    }
    if let Some(env) = p.env.as_ref() {
        let env_obj = env
            .iter()
            .map(|(key, value)| {
                (
                    key.clone(),
                    match value {
                        Some(value) => Value::String(value.clone()),
                        None => Value::Null,
                    },
                )
            })
            .collect();
        params.insert("env".to_owned(), Value::Object(env_obj));
    }
    if let Some(size) = p.size {
        let mut size_obj = Map::<String, Value>::new();
        size_obj.insert(
            "rows".to_owned(),
            Value::Number(serde_json::Number::from(size.rows)),
        );
        size_obj.insert(
            "cols".to_owned(),
            Value::Number(serde_json::Number::from(size.cols)),
        );
        params.insert("size".to_owned(), Value::Object(size_obj));
    }
    if let Some(sandbox_policy) = p.sandbox_policy.as_ref() {
        params.insert(
            "sandboxPolicy".to_owned(),
            sandbox_policy_to_wire_value(sandbox_policy),
        );
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

fn insert_privileged_escalation_approved(params: &mut Map<String, Value>, approved: bool) {
    if approved {
        params.insert("privilegedEscalationApproved".to_owned(), Value::Bool(true));
    }
}

// ── Prompt → thread/turn param transformations ────────────────────────────
// Pure functions: no self, no side effects. Allocation: one struct per call.

use super::{PromptRunParams, ReasoningEffort};

/// Build ThreadStartParams from a prompt run request.
/// Allocation: String clones for model + cwd. Complexity: O(1).
pub(super) fn thread_start_params_from_prompt(p: &PromptRunParams) -> ThreadStartParams {
    ThreadStartParams {
        model: p.model.clone(),
        cwd: Some(p.cwd.clone()),
        approval_policy: Some(p.approval_policy),
        sandbox_policy: Some(p.sandbox_policy.clone()),
        privileged_escalation_approved: p.privileged_escalation_approved,
        ..ThreadStartParams::default()
    }
}

/// Build TurnStartParams from a prompt run request with explicit effort.
/// Allocation: Vec<InputItem> (O(n) attachments) + String clones. Complexity: O(n).
pub(super) fn turn_start_params_from_prompt(
    p: &PromptRunParams,
    effort: ReasoningEffort,
) -> TurnStartParams {
    TurnStartParams {
        input: build_prompt_inputs(&p.prompt, &p.attachments),
        cwd: Some(p.cwd.clone()),
        approval_policy: Some(p.approval_policy),
        sandbox_policy: Some(p.sandbox_policy.clone()),
        privileged_escalation_approved: p.privileged_escalation_approved,
        model: p.model.clone(),
        effort: Some(effort),
        summary: None,
        output_schema: p.output_schema.clone(),
        ..TurnStartParams::default()
    }
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;
    use serde_json::json;

    use super::deserialize_result;
    use crate::runtime::errors::RpcError;

    #[derive(Debug, Deserialize)]
    struct ExpectedResult {
        ok: bool,
    }

    #[test]
    fn deserialize_result_redacts_payload_values_on_parse_failure() {
        let err = deserialize_result::<ExpectedResult>(
            "thread/read",
            json!({
                "thread": {"id": "thr_1"},
                "assistantText": "secret-output"
            }),
        )
        .expect_err("parse must fail");

        let RpcError::InvalidRequest(message) = err else {
            panic!("expected invalid request");
        };
        assert!(message.contains("thread/read invalid result"));
        assert!(message.contains("response: object(keys=[assistantText,thread])"));
        assert!(!message.contains("secret-output"));
        assert!(!message.contains("thr_1"));
    }

    #[test]
    fn deserialize_result_succeeds_for_matching_shape() {
        let result = deserialize_result::<ExpectedResult>("echo/test", json!({"ok": true}))
            .expect("matching result");
        assert!(result.ok);
    }
}
