use serde_json::{json, Value};

use crate::runtime::errors::{RpcError, RuntimeError};
use crate::runtime::rpc_contract::methods;

pub(crate) fn validate_positive_capacity(name: &str, value: usize) -> Result<(), RuntimeError> {
    if value > 0 {
        return Ok(());
    }
    Err(RuntimeError::InvalidConfig(format!("{name} must be > 0")))
}

pub(crate) fn trim_tail_bytes(tail: &mut Vec<u8>, max_tail_bytes: usize) {
    if tail.len() <= max_tail_bytes {
        return;
    }
    let overflow = tail.len().saturating_sub(max_tail_bytes);
    tail.drain(..overflow);
}

pub(crate) fn normalize_text_tail(bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() {
        return None;
    }
    let text = String::from_utf8_lossy(bytes).to_string();
    let trimmed = text.trim_end_matches(['\n', '\r']).to_owned();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed)
}

pub(crate) fn trim_ascii_line_endings(mut raw: &[u8]) -> &[u8] {
    while let Some(last) = raw.last() {
        if *last == b'\n' || *last == b'\r' {
            raw = &raw[..raw.len() - 1];
            continue;
        }
        break;
    }
    raw
}

pub(crate) fn should_flush_after_n_events(pending_writes: u64, events: u64) -> bool {
    pending_writes >= events.max(1)
}

pub(crate) fn build_rpc_request(rpc_id: u64, method: &str, params: Value) -> Value {
    json!({
        "id": rpc_id,
        "method": method,
        "params": params
    })
}

pub(crate) fn compute_deadline_millis(now_millis: i64, timeout_ms: u64) -> i64 {
    let timeout_i64 = i64::try_from(timeout_ms).unwrap_or(i64::MAX);
    now_millis.saturating_add(timeout_i64)
}

pub(crate) fn timeout_result_payload(method: &str, cancel: bool) -> Value {
    match method {
        methods::ITEM_TOOL_REQUEST_USER_INPUT => json!({ "answers": {} }),
        methods::ITEM_TOOL_CALL => json!({ "success": false, "contentItems": [] }),
        _ => {
            let decision = if cancel { "cancel" } else { "decline" };
            json!({ "decision": decision })
        }
    }
}

pub(crate) fn timeout_error_payload(method: &str) -> Value {
    json!({
        "code": -32000,
        "message": "server request timed out",
        "data": { "method": method }
    })
}

pub(crate) enum PendingRpcOutcome {
    Ready(Result<Value, RpcError>),
    Timeout,
}

pub(crate) fn project_pending_rpc_outcome(
    waited: Result<
        Result<Result<Value, RpcError>, tokio::sync::oneshot::error::RecvError>,
        tokio::time::error::Elapsed,
    >,
) -> PendingRpcOutcome {
    match waited {
        Ok(Ok(result)) => PendingRpcOutcome::Ready(result),
        Ok(Err(_)) => PendingRpcOutcome::Ready(Err(RpcError::TransportClosed)),
        Err(_) => PendingRpcOutcome::Timeout,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trim_ascii_line_endings_removes_crlf_suffix_only() {
        assert_eq!(trim_ascii_line_endings(b"abc\r\n"), b"abc");
        assert_eq!(trim_ascii_line_endings(b"abc"), b"abc");
        assert_eq!(trim_ascii_line_endings(b"\r\n"), b"");
    }

    #[test]
    fn normalize_text_tail_returns_none_for_empty_or_newlines_only() {
        assert_eq!(normalize_text_tail(b""), None);
        assert_eq!(normalize_text_tail(b"\n\r"), None);
        assert_eq!(normalize_text_tail(b"hello\n"), Some("hello".to_owned()));
    }

    #[test]
    fn trim_tail_bytes_keeps_latest_segment() {
        let mut bytes = b"abcdef".to_vec();
        trim_tail_bytes(&mut bytes, 3);
        assert_eq!(bytes, b"def");
    }

    #[test]
    fn should_flush_after_n_events_applies_minimum_one() {
        assert!(!should_flush_after_n_events(0, 0));
        assert!(should_flush_after_n_events(1, 0));
        assert!(!should_flush_after_n_events(1, 2));
        assert!(should_flush_after_n_events(2, 2));
    }

    #[test]
    fn compute_deadline_millis_saturates_timeout_cast_overflow() {
        let deadline = compute_deadline_millis(123, u64::MAX);
        assert_eq!(deadline, i64::MAX);
    }

    #[test]
    fn compute_deadline_millis_saturates_add_overflow() {
        let deadline = compute_deadline_millis(i64::MAX - 5, 10);
        assert_eq!(deadline, i64::MAX);
    }

    #[test]
    fn compute_deadline_millis_preserves_normal_case() {
        let deadline = compute_deadline_millis(10_000, 250);
        assert_eq!(deadline, 10_250);
    }

    #[test]
    fn timeout_result_payload_uses_method_specific_shape() {
        let user_input = timeout_result_payload(methods::ITEM_TOOL_REQUEST_USER_INPUT, false);
        assert_eq!(user_input["answers"], json!({}));

        let tool_call = timeout_result_payload(methods::ITEM_TOOL_CALL, true);
        assert_eq!(tool_call["success"], json!(false));
        assert_eq!(tool_call["contentItems"], json!([]));

        let decline = timeout_result_payload("item/commandExecutionRequest/approval", false);
        assert_eq!(decline["decision"], "decline");

        let cancel = timeout_result_payload("item/commandExecutionRequest/approval", true);
        assert_eq!(cancel["decision"], "cancel");
    }

    #[test]
    fn timeout_error_payload_preserves_method_context() {
        let payload = timeout_error_payload(methods::ACCOUNT_CHATGPT_AUTH_TOKENS_REFRESH);
        assert_eq!(payload["code"], json!(-32000));
        assert_eq!(payload["message"], json!("server request timed out"));
        assert_eq!(
            payload["data"]["method"],
            json!(methods::ACCOUNT_CHATGPT_AUTH_TOKENS_REFRESH)
        );
    }
}
