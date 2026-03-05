use crate::runtime::events::{Direction, Envelope, MsgKind};
use serde::Serialize;
use serde_json::{json, Value};

use super::{ApprovalResponsePayload, WebError};

/// Validate and normalize incoming turn payload.
/// Side effects: none. Allocation: None (mutates in place). Complexity: O(1).
pub(super) fn normalize_turn_start_params(
    thread_id: &str,
    mut task: Value,
) -> Result<Value, WebError> {
    let obj = task.as_object_mut().ok_or(WebError::InvalidTurnPayload)?;

    if let Some(existing_thread_id) = obj.get("threadId") {
        match existing_thread_id {
            Value::String(value) if value == thread_id => {}
            _ => return Err(WebError::Forbidden),
        }
    }
    obj.insert("threadId".to_owned(), Value::String(thread_id.to_owned()));
    Ok(task)
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SseEnvelope<'a> {
    seq: u64,
    ts_millis: i64,
    direction: &'a Direction,
    kind: &'a MsgKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    method: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thread_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    turn_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    item_id: Option<&'a str>,
    json: SseJsonPayload<'a>,
}

enum SseJsonPayload<'a> {
    Direct(&'a std::sync::Arc<Value>),
    Redacted(Value),
}

impl<'a> serde::Serialize for SseJsonPayload<'a> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match self {
            SseJsonPayload::Direct(v) => v.serialize(serializer),
            SseJsonPayload::Redacted(v) => v.serialize(serializer),
        }
    }
}

pub(super) fn serialize_sse_envelope(envelope: &Envelope) -> Result<String, WebError> {
    let json_payload = match envelope.kind {
        MsgKind::Response | MsgKind::Unknown => {
            if let Some(obj) = envelope.json.as_object() {
                if obj.contains_key("id") {
                    let mut redacted = (*envelope.json).clone();
                    if let Some(mut_obj) = redacted.as_object_mut() {
                        mut_obj.remove("id");
                    }
                    SseJsonPayload::Redacted(redacted)
                } else {
                    SseJsonPayload::Direct(&envelope.json)
                }
            } else {
                SseJsonPayload::Direct(&envelope.json)
            }
        }
        _ => SseJsonPayload::Direct(&envelope.json),
    };

    let sse = SseEnvelope {
        seq: envelope.seq,
        ts_millis: envelope.ts_millis,
        direction: &envelope.direction,
        kind: &envelope.kind,
        method: envelope.method.as_deref(),
        thread_id: envelope.thread_id.as_deref(),
        turn_id: envelope.turn_id.as_deref(),
        item_id: envelope.item_id.as_deref(),
        json: json_payload,
    };

    let json_str = serde_json::to_string(&sse).map_err(|e| WebError::Internal(e.to_string()))?;
    Ok(format!("data: {json_str}\n\n"))
}

/// Adapter-local parser for server-request thread id.
/// Accepts either `threadId` or nested `thread.id` shapes.
pub(super) fn extract_thread_id_from_server_request_params(params: &Value) -> Option<String> {
    parse_str_field(params, "threadId")
        .or_else(|| parse_nested_id_field(params, "thread"))
        .map(ToOwned::to_owned)
}

/// Adapter-local parser for turn/start result turn id.
/// Accepts `{turn:{id}}`, `turnId`, top-level `id`, or raw string payload.
pub(super) fn parse_turn_id_from_turn_result(value: &Value) -> Option<String> {
    value
        .pointer("/turn/id")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| {
            value
                .get("turnId")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .or_else(|| {
            value
                .get("id")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .or_else(|| value.as_str().map(ToOwned::to_owned))
}

fn parse_str_field<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str).or_else(|| {
        value
            .get("params")
            .and_then(|v| v.get(key))
            .and_then(Value::as_str)
    })
}

fn parse_nested_id_field<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value
        .get(key)
        .and_then(|v| v.get("id"))
        .and_then(Value::as_str)
        .or_else(|| {
            value
                .get("params")
                .and_then(|v| v.get(key))
                .and_then(|v| v.get("id"))
                .and_then(Value::as_str)
        })
}
