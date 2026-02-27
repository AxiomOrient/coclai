use serde_json::Value;

use crate::errors::{RpcError, RpcErrorObject};
use crate::events::{JsonRpcId, MsgKind};

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
    pub method: Option<String>,
    pub thread_id: Option<String>,
    pub turn_id: Option<String>,
    pub item_id: Option<String>,
}

/// Classify a raw JSON message with constant-time key presence checks.
/// Allocation: none. Complexity: O(1).
pub fn classify_message(json: &Value) -> MsgKind {
    let has_id = json.get("id").is_some();
    let has_method = json.get("method").is_some();
    let has_result = json.get("result").is_some();
    let has_error = json.get("error").is_some();

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

/// Best-effort identifier extraction from known shallow JSON-RPC slots.
/// Allocation: up to 3 Strings (only when ids exist). Complexity: O(1).
pub fn extract_ids(json: &Value) -> ExtractedIds {
    let roots = [
        json.get("params"),
        json.get("result"),
        json.get("error").and_then(|e| e.get("data")),
        Some(json),
    ];

    let thread_id = roots
        .iter()
        .copied()
        .flatten()
        .find_map(get_thread_id)
        .map(ToOwned::to_owned);

    let turn_id = roots
        .iter()
        .copied()
        .flatten()
        .find_map(get_turn_id)
        .map(ToOwned::to_owned);

    let item_id = roots
        .iter()
        .copied()
        .flatten()
        .find_map(get_item_id)
        .map(ToOwned::to_owned);

    ExtractedIds {
        thread_id,
        turn_id,
        item_id,
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
    let kind = if has_id && !has_method && (has_result || has_error) {
        MsgKind::Response
    } else if has_id && has_method && !has_result && !has_error {
        MsgKind::ServerRequest
    } else if has_method && !has_id {
        MsgKind::Notification
    } else {
        MsgKind::Unknown
    };

    let method = method_value.and_then(Value::as_str).map(ToOwned::to_owned);
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
            thread_id = get_thread_id(root).map(ToOwned::to_owned);
        }
        if turn_id.is_none() {
            turn_id = get_turn_id(root).map(ToOwned::to_owned);
        }
        if item_id.is_none() {
            item_id = get_item_id(root).map(ToOwned::to_owned);
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

fn get_str_field<'a>(root: &'a Value, key: &str) -> Option<&'a str> {
    if let Some(s) = root.get(key).and_then(Value::as_str) {
        return Some(s);
    }
    root.get("params")
        .and_then(|v| v.get(key))
        .and_then(Value::as_str)
}

fn get_nested_id_field<'a>(root: &'a Value, key: &str) -> Option<&'a str> {
    root.get(key)
        .and_then(|v| v.get("id"))
        .and_then(Value::as_str)
        .or_else(|| {
            root.get("params")
                .and_then(|v| v.get(key))
                .and_then(|v| v.get("id"))
                .and_then(Value::as_str)
        })
}

fn get_thread_id(root: &Value) -> Option<&str> {
    get_str_field(root, "threadId").or_else(|| get_nested_id_field(root, "thread"))
}

fn get_turn_id(root: &Value) -> Option<&str> {
    get_str_field(root, "turnId").or_else(|| get_nested_id_field(root, "turn"))
}

fn get_item_id(root: &Value) -> Option<&str> {
    get_str_field(root, "itemId").or_else(|| get_nested_id_field(root, "item"))
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
            assert_eq!(meta.thread_id, ids.thread_id);
            assert_eq!(meta.turn_id, ids.turn_id);
            assert_eq!(meta.item_id, ids.item_id);
        }
    }
}
