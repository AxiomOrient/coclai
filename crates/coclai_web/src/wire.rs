use coclai_runtime::events::Envelope;
use serde_json::{json, Value};

use super::{ApprovalResponsePayload, WebError};

/// Validate and normalize incoming turn payload.
/// Side effects: none. Allocation: one object clone. Complexity: O(n), n = property count.
pub(super) fn normalize_turn_start_params(
    thread_id: &str,
    task: &Value,
) -> Result<Value, WebError> {
    let mut obj = task
        .as_object()
        .cloned()
        .ok_or(WebError::InvalidTurnPayload)?;

    if let Some(existing_thread_id) = obj.get("threadId") {
        match existing_thread_id {
            Value::String(value) if value == thread_id => {}
            _ => return Err(WebError::Forbidden),
        }
    }
    obj.insert("threadId".to_owned(), Value::String(thread_id.to_owned()));
    Ok(Value::Object(obj))
}

impl ApprovalResponsePayload {
    pub(super) fn into_result_payload(self) -> Result<Value, WebError> {
        if let Some(result) = self.result {
            return Ok(result);
        }
        if let Some(decision) = self.decision {
            return Ok(json!({ "decision": decision }));
        }
        Err(WebError::InvalidApprovalPayload)
    }
}

pub(super) fn serialize_sse_envelope(envelope: &Envelope) -> Result<String, WebError> {
    let mut value =
        serde_json::to_value(envelope).map_err(|e| WebError::Internal(e.to_string()))?;
    redact_internal_identifiers(&mut value);
    serde_json::to_string(&value)
        .map(|json| format!("data: {json}\n\n"))
        .map_err(|e| WebError::Internal(e.to_string()))
}

fn redact_internal_identifiers(value: &mut Value) {
    let Some(obj) = value.as_object_mut() else {
        return;
    };
    obj.remove("rpcId");
    let kind = obj.get("kind").and_then(Value::as_str);
    if matches!(kind, Some("response" | "unknown")) {
        if let Some(json_obj) = obj.get_mut("json").and_then(Value::as_object_mut) {
            json_obj.remove("id");
        }
    }
}
