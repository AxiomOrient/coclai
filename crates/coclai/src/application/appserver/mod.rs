use serde_json::{json, Map, Value};
use tokio::sync::mpsc::error::TryRecvError;

use crate::errors::RpcErrorObject;
use crate::ServerRequestRx;

pub fn resolve_server_request_take_limit(payload: &Map<String, Value>) -> usize {
    payload
        .get("max_items")
        .and_then(Value::as_u64)
        .unwrap_or(16)
        .min(256) as usize
}

pub fn render_request_result(connection_id: &str, method: &str, result: Value) -> Value {
    json!({
        "connection_id": connection_id,
        "method": method,
        "result": result,
    })
}

pub fn render_notify_result(connection_id: &str, method: &str) -> Value {
    json!({
        "connection_id": connection_id,
        "method": method,
        "ok": true,
    })
}

pub fn collect_server_requests(
    rx: &mut ServerRequestRx,
    max_items: usize,
) -> Result<(Vec<Value>, bool), String> {
    let mut items = Vec::new();
    let mut disconnected = false;

    for _ in 0..max_items {
        match rx.try_recv() {
            Ok(item) => {
                let value = serde_json::to_value(item)
                    .map_err(|err| format!("failed to encode server request: {err}"))?;
                items.push(value);
            }
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => {
                disconnected = true;
                break;
            }
        }
    }

    Ok((items, disconnected))
}

pub fn render_server_request_take_result(
    connection_id: &str,
    items: Vec<Value>,
    disconnected: bool,
) -> Value {
    json!({
        "connection_id": connection_id,
        "items": items,
        "disconnected": disconnected,
    })
}

pub fn render_server_request_ack(connection_id: &str, approval_id: &str) -> Value {
    json!({
        "connection_id": connection_id,
        "approval_id": approval_id,
        "ok": true,
    })
}

pub fn parse_rpc_error_object(error_obj: &Map<String, Value>) -> Result<RpcErrorObject, String> {
    let code = error_obj
        .get("code")
        .and_then(Value::as_i64)
        .ok_or_else(|| "payload.error.code must be an integer".to_owned())?;
    let message = error_obj
        .get("message")
        .and_then(Value::as_str)
        .ok_or_else(|| "payload.error.message must be a string".to_owned())?;
    let data = error_obj.get("data").cloned();
    Ok(RpcErrorObject {
        code,
        message: message.to_owned(),
        data,
    })
}

pub fn render_rpc_forward_result(connection_id: &str, method: &str, result: Value) -> Value {
    json!({
        "connection_id": connection_id,
        "method": method,
        "result": result,
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{parse_rpc_error_object, resolve_server_request_take_limit};

    #[test]
    fn take_limit_is_bounded() {
        assert_eq!(
            resolve_server_request_take_limit(json!({}).as_object().unwrap()),
            16
        );
        assert_eq!(
            resolve_server_request_take_limit(json!({"max_items": 8}).as_object().unwrap()),
            8
        );
        assert_eq!(
            resolve_server_request_take_limit(json!({"max_items": 999}).as_object().unwrap()),
            256
        );
    }

    #[test]
    fn rpc_error_object_requires_code_and_message() {
        let err = parse_rpc_error_object(&json!({"message": "x"}).as_object().unwrap().clone())
            .expect_err("missing code must fail");
        assert_eq!(err, "payload.error.code must be an integer");

        let err = parse_rpc_error_object(&json!({"code": 1}).as_object().unwrap().clone())
            .expect_err("missing message must fail");
        assert_eq!(err, "payload.error.message must be a string");
    }
}
