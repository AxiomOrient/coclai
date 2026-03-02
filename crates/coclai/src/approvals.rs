use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ServerRequest {
    pub approval_id: String,
    pub method: String,
    pub params: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PendingServerRequest {
    pub approval_id: String,
    pub deadline_unix_ms: i64,
    pub method: String,
    pub params: Value,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TimeoutAction {
    Decline,
    Cancel,
    Error,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ServerRequestConfig {
    pub default_timeout_ms: u64,
    pub on_timeout: TimeoutAction,
    pub auto_decline_unknown: bool,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ServerRequestRoute {
    Queue,
    AutoDecline,
}

impl Default for ServerRequestConfig {
    fn default() -> Self {
        Self {
            default_timeout_ms: 30_000,
            on_timeout: TimeoutAction::Decline,
            auto_decline_unknown: true,
        }
    }
}

/// Pure classifier for known server-request methods.
/// Allocation: none. Complexity: O(1).
pub fn is_known_server_request_method(method: &str) -> bool {
    matches!(
        method,
        "item/commandExecution/requestApproval"
            | "item/fileChange/requestApproval"
            | "item/tool/requestUserInput"
            | "item/tool/call"
            | "account/chatgptAuthTokens/refresh"
    )
}

fn is_legacy_server_request_method(method: &str) -> bool {
    matches!(method, "applyPatchApproval" | "execCommandApproval")
}

/// Decide whether a server request should be queued or auto-declined.
/// Allocation: none. Complexity: O(1).
pub fn route_server_request(method: &str, auto_decline_unknown: bool) -> ServerRequestRoute {
    if is_legacy_server_request_method(method) {
        return ServerRequestRoute::AutoDecline;
    }

    if auto_decline_unknown && !is_known_server_request_method(method) {
        ServerRequestRoute::AutoDecline
    } else {
        ServerRequestRoute::Queue
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_known_method_to_queue() {
        let route = route_server_request("item/fileChange/requestApproval", true);
        assert_eq!(route, ServerRequestRoute::Queue);
    }

    #[test]
    fn routes_known_dynamic_tool_call_method_to_queue() {
        let route = route_server_request("item/tool/call", true);
        assert_eq!(route, ServerRequestRoute::Queue);
    }

    #[test]
    fn routes_known_auth_refresh_method_to_queue() {
        let route = route_server_request("account/chatgptAuthTokens/refresh", true);
        assert_eq!(route, ServerRequestRoute::Queue);
    }

    #[test]
    fn routes_unknown_method_to_auto_decline_when_enabled() {
        let route = route_server_request("item/unknown/requestApproval", true);
        assert_eq!(route, ServerRequestRoute::AutoDecline);
    }

    #[test]
    fn routes_unknown_method_to_queue_when_auto_decline_disabled() {
        let route = route_server_request("item/unknown/requestApproval", false);
        assert_eq!(route, ServerRequestRoute::Queue);
    }

    #[test]
    fn routes_legacy_method_to_auto_decline_even_when_unknown_queue_enabled() {
        let route = route_server_request("applyPatchApproval", false);
        assert_eq!(route, ServerRequestRoute::AutoDecline);
    }
}
