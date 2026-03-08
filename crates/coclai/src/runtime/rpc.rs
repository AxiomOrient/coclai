use serde_json::Value;
use std::sync::Arc;

use crate::runtime::errors::{RpcError, RpcErrorObject};
use crate::runtime::events::{JsonRpcId, MsgKind};
use crate::runtime::id::{extract_item_id, extract_thread_id, extract_turn_id};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtractedIds {
    pub thread_id: Option<String>,
    pub turn_id: Option<String>,
    pub item_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MsgMetadata {
    pub kind: MsgKind,
    pub response_id: Option<u64>,
    pub rpc_id: Option<JsonRpcId>,
    pub method: Option<Arc<str>>,
    pub thread_id: Option<Arc<str>>,
    pub turn_id: Option<Arc<str>>,
    pub item_id: Option<Arc<str>>,
}

/// Classify a raw JSON message with constant-time key presence checks.
/// Allocation: none. Complexity: O(1).
pub fn classify_message(json: &Value) -> MsgKind {
    let has_id = json.get("id").is_some();
    let has_method = json.get("method").is_some();
    let has_result = json.get("result").is_some();
    let has_error = json.get("error").is_some();

    classify_jsonrpc_shape(has_id, has_method, has_result, has_error)
}

/// Best-effort identifier extraction from known shallow JSON-RPC slots.
/// Allocation: up to 3 Strings (only when ids exist). Complexity: O(1).
pub fn extract_ids(json: &Value) -> ExtractedIds {
    let meta = extract_message_metadata(json);

    ExtractedIds {
        thread_id: meta.thread_id.map(|s| s.to_string()),
        turn_id: meta.turn_id.map(|s| s.to_string()),
        item_id: meta.item_id.map(|s| s.to_string()),
    }
}

/// Extract commonly used dispatch metadata in one pass over top-level keys.
/// Allocation: owned method/id strings only when present. Complexity: O(1).
pub fn extract_message_metadata(json: &Value) -> MsgMetadata {
    let obj = json.as_object();
    let id_value = obj.and_then(|value| value.get("id"));
    let method_value = obj.and_then(|value| value.get("method"));
    let result_value = obj.and_then(|value| value.get("result"));
    let error_value = obj.and_then(|value| value.get("error"));

    let has_id = id_value.is_some();
    let has_method = method_value.is_some();
    let has_result = result_value.is_some();
    let has_error = error_value.is_some();
    let kind = classify_jsonrpc_shape(has_id, has_method, has_result, has_error);

    let method = method_value.and_then(Value::as_str).map(Arc::from);
    let response_id = parse_response_rpc_id_value(id_value);
    let rpc_id = parse_jsonrpc_id_value(id_value);

    let roots = [
        obj.and_then(|value| value.get("params")),
        result_value,
        error_value.and_then(|value| value.get("data")),
        Some(json),
    ];

    let mut thread_id = None;
    let mut turn_id = None;
    let mut item_id = None;
    for root in roots.into_iter().flatten() {
        if thread_id.is_none() {
            thread_id = extract_thread_id(root).map(Arc::from);
        }
        if turn_id.is_none() {
            turn_id = extract_turn_id(root).map(Arc::from);
        }
        if item_id.is_none() {
            item_id = extract_item_id(root).map(Arc::from);
        }
        if thread_id.is_some() && turn_id.is_some() && item_id.is_some() {
            break;
        }
    }

    MsgMetadata {
        kind,
        response_id,
        rpc_id,
        method,
        thread_id,
        turn_id,
        item_id,
    }
}

/// Map a JSON-RPC error object into a typed error enum.
/// Allocation: message clone + optional data clone. Complexity: O(1).
pub fn map_rpc_error(json_error: &Value) -> RpcError {
    let code = json_error.get("code").and_then(Value::as_i64);
    let message = json_error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("unknown rpc error")
        .to_owned();
    let data = json_error.get("data").cloned();

    match code {
        Some(-32001) => RpcError::Overloaded,
        Some(-32600) => RpcError::InvalidRequest(message),
        Some(-32601) => RpcError::MethodNotFound(message),
        Some(code) => RpcError::ServerError(RpcErrorObject {
            code,
            message,
            data,
        }),
        None => RpcError::InvalidRequest("invalid rpc error payload".to_owned()),
    }
}

fn parse_response_rpc_id_value(id_value: Option<&Value>) -> Option<u64> {
    match id_value {
        Some(Value::Number(number)) => number.as_u64(),
        Some(Value::String(text)) => text.parse::<u64>().ok(),
        _ => None,
    }
}

fn parse_jsonrpc_id_value(id_value: Option<&Value>) -> Option<JsonRpcId> {
    match id_value {
        Some(Value::Number(number)) => number.as_u64().map(JsonRpcId::Number),
        Some(Value::String(text)) => Some(JsonRpcId::Text(text.clone())),
        _ => None,
    }
}

fn classify_jsonrpc_shape(
    has_id: bool,
    has_method: bool,
    has_result: bool,
    has_error: bool,
) -> MsgKind {
    if has_id && !has_method && (has_result || has_error) {
        return MsgKind::Response;
    }
    if has_id && has_method && !has_result && !has_error {
        return MsgKind::ServerRequest;
    }
    if has_method && !has_id {
        return MsgKind::Notification;
    }
    MsgKind::Unknown
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn classify_response() {
        let v = json!({"id":1,"result":{}});
        assert_eq!(classify_message(&v), MsgKind::Response);
    }

    #[test]
    fn classify_server_request() {
        let v = json!({"id":2,"method":"item/fileChange/requestApproval","params":{}});
        assert_eq!(classify_message(&v), MsgKind::ServerRequest);
    }

    #[test]
    fn classify_notification() {
        let v = json!({"method":"turn/started","params":{}});
        assert_eq!(classify_message(&v), MsgKind::Notification);
    }

    #[test]
    fn classify_unknown() {
        let v = json!({"foo":"bar"});
        assert_eq!(classify_message(&v), MsgKind::Unknown);
    }

    #[test]
    fn extract_ids_prefers_params() {
        let v = json!({
            "params": {
                "threadId": "thr_1",
                "turnId": "turn_1",
                "itemId": "item_1"
            }
        });
        let ids = extract_ids(&v);
        assert_eq!(ids.thread_id.as_deref(), Some("thr_1"));
        assert_eq!(ids.turn_id.as_deref(), Some("turn_1"));
        assert_eq!(ids.item_id.as_deref(), Some("item_1"));
    }

    #[test]
    fn extract_ids_supports_nested_struct_ids() {
        let v = json!({
            "params": {
                "thread": {"id": "thr_nested"},
                "turn": {"id": "turn_nested"},
                "item": {"id": "item_nested"}
            }
        });
        let ids = extract_ids(&v);
        assert_eq!(ids.thread_id.as_deref(), Some("thr_nested"));
        assert_eq!(ids.turn_id.as_deref(), Some("turn_nested"));
        assert_eq!(ids.item_id.as_deref(), Some("item_nested"));
    }

    #[test]
    fn extract_ids_ignores_legacy_conversation_id() {
        let v = json!({
            "params": {
                "conversationId": "thr_conv"
            }
        });
        let ids = extract_ids(&v);
        assert_eq!(ids.thread_id, None);
        assert_eq!(ids.turn_id, None);
        assert_eq!(ids.item_id, None);
    }

    #[test]
    fn extract_ids_rejects_non_canonical_id_values() {
        let v = json!({
            "params": {
                "threadId": " thr_space ",
                "turn": {"id": ""},
                "itemId": "item_ok"
            }
        });
        let ids = extract_ids(&v);
        assert_eq!(ids.thread_id, None);
        assert_eq!(ids.turn_id, None);
        assert_eq!(ids.item_id.as_deref(), Some("item_ok"));
    }

    #[test]
    fn map_overloaded_error() {
        let v = json!({"code": -32001, "message": "ingress overload"});
        assert_eq!(map_rpc_error(&v), RpcError::Overloaded);
    }

    #[test]
    fn extract_message_metadata_matches_legacy_helpers() {
        let fixtures = vec![
            json!({
                "id": 1,
                "result": {
                    "thread": {"id": "thr_1"},
                    "turn": {"id": "turn_1"},
                    "item": {"id": "item_1"}
                }
            }),
            json!({
                "id": "42",
                "method": "item/fileChange/requestApproval",
                "params": {
                    "threadId": "thr_2",
                    "turnId": "turn_2",
                    "itemId": "item_2"
                }
            }),
            json!({
                "method": "turn/started",
                "params": {
                    "thread": {"id": "thr_3"},
                    "turn": {"id": "turn_3"}
                }
            }),
        ];

        for fixture in fixtures {
            let meta = extract_message_metadata(&fixture);
            let ids = extract_ids(&fixture);

            assert_eq!(meta.kind, classify_message(&fixture));
            assert_eq!(meta.thread_id.map(|s| s.to_string()), ids.thread_id);
            assert_eq!(meta.turn_id.map(|s| s.to_string()), ids.turn_id);
            assert_eq!(meta.item_id.map(|s| s.to_string()), ids.item_id);
        }
    }
}
