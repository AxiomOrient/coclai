/// Pre-tool-use approval loop: intercepts approval requests and runs hooks.
///
/// When pre_tool_use_hooks are registered, this module manages the approval channel
/// internally. commandExecution and fileChange approvals are routed through hooks;
/// other server requests (ITEM_TOOL_CALL, ITEM_TOOL_REQUEST_USER_INPUT) are consumed
/// from the rx but left to the dispatcher's timeout-sweep to resolve.
use serde_json::{json, Value};

use crate::plugin::{HookContext, HookPhase, HookReport};
use crate::runtime::approvals::ServerRequest;
use crate::runtime::core::Runtime;
use crate::runtime::now_millis;
use crate::runtime::rpc_contract::methods;

/// Extract a human-readable tool name from an approval request.
/// Pure function; no I/O.
/// Allocation: at most one String. Complexity: O(1).
pub(crate) fn extract_tool_name(req: &ServerRequest) -> Option<String> {
    match req.method.as_str() {
        methods::ITEM_COMMAND_EXECUTION_REQUEST_APPROVAL => {
            // Extract just the binary name from the full command string.
            req.params
                .get("command")
                .and_then(|v| v.as_str())
                .and_then(|cmd| cmd.split_whitespace().next())
                .map(ToOwned::to_owned)
        }
        methods::ITEM_FILE_CHANGE_REQUEST_APPROVAL => Some("file_change".to_owned()),
        methods::ITEM_TOOL_CALL => req
            .params
            .get("toolName")
            .and_then(|v| v.as_str())
            .map(ToOwned::to_owned),
        _ => None,
    }
}

/// Extract tool input from an approval request for hook context.
/// Pure function; no I/O.
/// Allocation: clones the params Value. Complexity: O(n), n = params depth.
pub(crate) fn extract_tool_input(req: &ServerRequest) -> Option<Value> {
    if req.params.is_null() {
        None
    } else {
        Some(req.params.clone())
    }
}

/// Background approval loop. Takes exclusive ownership of server_request_rx.
/// For commandExecution/fileChange approvals: runs pre_tool_use_hooks, responds accept/decline.
/// For all other server requests: drains from rx without responding (timeout-sweep handles them).
/// Loop exits when the runtime shuts down and the channel closes.
/// Allocation: one HookContext + one JSON Value per approval. Complexity: O(hooks * requests).
pub(crate) async fn run_tool_use_approval_loop(runtime: Runtime) {
    let mut rx = match runtime.take_server_request_rx().await {
        Ok(rx) => rx,
        Err(_) => {
            tracing::warn!("tool-use approval loop: server_request_rx already taken; loop skipped");
            return;
        }
    };

    while let Some(req) = rx.recv().await {
        handle_approval_request(&runtime, req).await;
    }
}

/// Dispatch one server request from the approval channel.
///
/// commandExecution/fileChange approvals → run pre-tool-use hooks, respond accept/decline.
///
/// All other methods (ITEM_TOOL_CALL, ITEM_TOOL_REQUEST_USER_INPUT, etc.) are drained
/// from the rx channel without responding here. The dispatcher's timeout-sweep
/// (running every 50 ms) holds the corresponding entry in `pending_server_requests`
/// and will respond via the configured `TimeoutAction` after the deadline.
/// This split avoids conflicting responses between this loop and the dispatcher.
///
/// Allocation: one HookContext per approval call. Complexity: O(hooks).
async fn handle_approval_request(runtime: &Runtime, req: ServerRequest) {
    match req.method.as_str() {
        methods::ITEM_COMMAND_EXECUTION_REQUEST_APPROVAL
        | methods::ITEM_FILE_CHANGE_REQUEST_APPROVAL => {
            run_pre_tool_use_for_approval(runtime, &req).await;
        }
        _ => {
            // Intentionally drained; timeout-sweep responds to the pending_server_requests entry.
        }
    }
}

/// Run pre-tool-use hooks for one commandExecution/fileChange approval.
/// Responds accept on Noop/Mutate, decline on Block. Issues are published to the hook report.
/// Allocation: one HookContext + one HookReport + one JSON payload. Complexity: O(hooks).
async fn run_pre_tool_use_for_approval(runtime: &Runtime, req: &ServerRequest) {
    let tool_name = extract_tool_name(req);
    let tool_input = extract_tool_input(req);

    let ctx = HookContext {
        phase: HookPhase::PreToolUse,
        thread_id: req
            .params
            .get("threadId")
            .and_then(|value| value.as_str())
            .map(ToOwned::to_owned),
        turn_id: None,
        cwd: None,
        model: None,
        main_status: None,
        correlation_id: format!("tu-{}", uuid::Uuid::new_v4()),
        ts_ms: now_millis(),
        metadata: serde_json::Value::Null,
        tool_name,
        tool_input,
    };

    let mut report = HookReport::default();
    let decision = runtime.run_pre_tool_use_hooks(&ctx, &mut report).await;

    if !report.is_clean() {
        runtime.publish_hook_report(report);
    }

    let result = match decision {
        Ok(()) => json!({"decision": "accept"}),
        Err(_block_reason) => json!({"decision": "decline"}),
    };

    let _ = runtime.respond_approval_ok(&req.approval_id, result).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::approvals::ServerRequest;
    use serde_json::json;

    fn make_req(method: &str, params: Value) -> ServerRequest {
        ServerRequest {
            approval_id: "test-id".to_owned(),
            method: method.to_owned(),
            params,
        }
    }

    #[test]
    fn extracts_binary_name_from_command() {
        let req = make_req(
            methods::ITEM_COMMAND_EXECUTION_REQUEST_APPROVAL,
            json!({"command": "cargo test --lib"}),
        );
        assert_eq!(extract_tool_name(&req), Some("cargo".to_owned()));
    }

    #[test]
    fn extracts_file_change_tool_name() {
        let req = make_req(
            methods::ITEM_FILE_CHANGE_REQUEST_APPROVAL,
            json!({"path": "/foo/bar.rs"}),
        );
        assert_eq!(extract_tool_name(&req), Some("file_change".to_owned()));
    }

    #[test]
    fn extracts_tool_call_name() {
        let req = make_req(methods::ITEM_TOOL_CALL, json!({"toolName": "search_files"}));
        assert_eq!(extract_tool_name(&req), Some("search_files".to_owned()));
    }

    #[test]
    fn returns_none_for_unknown_method() {
        let req = make_req("item/unknown/method", json!({}));
        assert_eq!(extract_tool_name(&req), None);
    }

    #[test]
    fn extracts_tool_input_non_null() {
        let req = make_req(
            methods::ITEM_COMMAND_EXECUTION_REQUEST_APPROVAL,
            json!({"command": "ls"}),
        );
        assert_eq!(extract_tool_input(&req), Some(json!({"command": "ls"})));
    }

    #[test]
    fn returns_none_for_null_params() {
        let req = make_req(
            methods::ITEM_COMMAND_EXECUTION_REQUEST_APPROVAL,
            Value::Null,
        );
        assert_eq!(extract_tool_input(&req), None);
    }
}
