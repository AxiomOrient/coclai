use serde_json::Value;

use crate::runtime::rpc_contract::methods;

/// Extract a human-readable tool name from an approval request.
/// Pure function; no I/O.
/// Allocation: at most one String. Complexity: O(1).
pub(crate) fn extract_tool_name(method: &str, params: &Value) -> Option<String> {
    match method {
        methods::ITEM_COMMAND_EXECUTION_REQUEST_APPROVAL => {
            // Extract just the binary name from the full command string.
            params
                .get("command")
                .and_then(|v| v.as_str())
                .and_then(|cmd| cmd.split_whitespace().next())
                .map(ToOwned::to_owned)
        }
        methods::ITEM_FILE_CHANGE_REQUEST_APPROVAL => Some("file_change".to_owned()),
        methods::ITEM_TOOL_CALL => params
            .get("toolName")
            .and_then(|v| v.as_str())
            .map(ToOwned::to_owned),
        _ => None,
    }
}

/// Extract tool input from an approval request for hook context.
/// Pure function; no I/O.
/// Allocation: clones the params Value. Complexity: O(n), n = params depth.
pub(crate) fn extract_tool_input(params: &Value) -> Option<Value> {
    if params.is_null() {
        None
    } else {
        Some(params.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extracts_binary_name_from_command() {
        assert_eq!(
            extract_tool_name(
                methods::ITEM_COMMAND_EXECUTION_REQUEST_APPROVAL,
                &json!({"command": "cargo test --lib"}),
            ),
            Some("cargo".to_owned())
        );
    }

    #[test]
    fn extracts_file_change_tool_name() {
        assert_eq!(
            extract_tool_name(
                methods::ITEM_FILE_CHANGE_REQUEST_APPROVAL,
                &json!({"path": "/foo/bar.rs"}),
            ),
            Some("file_change".to_owned())
        );
    }

    #[test]
    fn extracts_tool_call_name() {
        assert_eq!(
            extract_tool_name(
                methods::ITEM_TOOL_CALL,
                &json!({"toolName": "search_files"})
            ),
            Some("search_files".to_owned())
        );
    }

    #[test]
    fn returns_none_for_unknown_method() {
        assert_eq!(extract_tool_name("item/unknown/method", &json!({})), None);
    }

    #[test]
    fn extracts_tool_input_non_null() {
        assert_eq!(
            extract_tool_input(&json!({"command": "ls"})),
            Some(json!({"command": "ls"}))
        );
    }

    #[test]
    fn returns_none_for_null_params() {
        assert_eq!(extract_tool_input(&Value::Null), None);
    }
}
