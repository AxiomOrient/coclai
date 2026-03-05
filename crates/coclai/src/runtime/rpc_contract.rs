use serde_json::Value;

use crate::runtime::errors::RpcError;
use crate::runtime::turn_output::{parse_thread_id, parse_turn_id};

/// Canonical method catalog shared by facade constants and known-method validation.
pub mod methods {
    pub const THREAD_START: &str = "thread/start";
    pub const THREAD_RESUME: &str = "thread/resume";
    pub const THREAD_FORK: &str = "thread/fork";
    pub const THREAD_ARCHIVE: &str = "thread/archive";
    pub const THREAD_READ: &str = "thread/read";
    pub const THREAD_LIST: &str = "thread/list";
    pub const THREAD_LOADED_LIST: &str = "thread/loaded/list";
    pub const THREAD_ROLLBACK: &str = "thread/rollback";
    pub const TURN_START: &str = "turn/start";
    pub const TURN_INTERRUPT: &str = "turn/interrupt";

    // Server-request methods (runtime inbound requests requiring a client response)
    pub const ITEM_COMMAND_EXECUTION_REQUEST_APPROVAL: &str =
        "item/commandExecution/requestApproval";
    pub const ITEM_FILE_CHANGE_REQUEST_APPROVAL: &str = "item/fileChange/requestApproval";
    pub const ITEM_TOOL_REQUEST_USER_INPUT: &str = "item/tool/requestUserInput";
    pub const ITEM_TOOL_CALL: &str = "item/tool/call";
    pub const ACCOUNT_CHATGPT_AUTH_TOKENS_REFRESH: &str = "account/chatgptAuthTokens/refresh";

    // Server-pushed notification events (not client requests)
    pub const THREAD_STARTED: &str = "thread/started";
    pub const TURN_STARTED: &str = "turn/started";
    pub const TURN_COMPLETED: &str = "turn/completed";
    pub const TURN_FAILED: &str = "turn/failed";
    pub const TURN_CANCELLED: &str = "turn/cancelled";
    pub const TURN_INTERRUPTED: &str = "turn/interrupted";
    pub const TURN_DIFF_UPDATED: &str = "turn/diff/updated";
    pub const TURN_PLAN_UPDATED: &str = "turn/plan/updated";
    pub const ITEM_STARTED: &str = "item/started";
    pub const ITEM_AGENT_MESSAGE_DELTA: &str = "item/agentMessage/delta";
    pub const ITEM_COMMAND_EXECUTION_OUTPUT_DELTA: &str = "item/commandExecution/outputDelta";
    pub const ITEM_COMPLETED: &str = "item/completed";
    pub const APPROVAL_ACK: &str = "approval/ack";

    pub const KNOWN: [&str; 10] = [
        THREAD_START,
        THREAD_RESUME,
        THREAD_FORK,
        THREAD_ARCHIVE,
        THREAD_READ,
        THREAD_LIST,
        THREAD_LOADED_LIST,
        THREAD_ROLLBACK,
        TURN_START,
        TURN_INTERRUPT,
    ];
}

/// Validation mode for JSON-RPC data integrity checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum RpcValidationMode {
    /// Skip all contract checks.
    None,
    /// Validate only methods known to the current app-server contract.
    #[default]
    KnownMethods,
}

/// Request-shape rule for one RPC method contract descriptor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RpcRequestContract {
    Object,
    ThreadStart,
    ThreadId,
    ThreadIdAndTurnId,
}

/// Response-shape rule for one RPC method contract descriptor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RpcResponseContract {
    Object,
    ThreadId,
    TurnId,
    DataArray,
}

/// Single-source descriptor for one app-server RPC contract method.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RpcContractDescriptor {
    pub method: &'static str,
    pub request: RpcRequestContract,
    pub response: RpcResponseContract,
}

const RPC_CONTRACT_DESCRIPTORS: [RpcContractDescriptor; 10] = [
    RpcContractDescriptor {
        method: methods::THREAD_START,
        request: RpcRequestContract::ThreadStart,
        response: RpcResponseContract::ThreadId,
    },
    RpcContractDescriptor {
        method: methods::THREAD_RESUME,
        request: RpcRequestContract::ThreadId,
        response: RpcResponseContract::ThreadId,
    },
    RpcContractDescriptor {
        method: methods::THREAD_FORK,
        request: RpcRequestContract::ThreadId,
        response: RpcResponseContract::ThreadId,
    },
    RpcContractDescriptor {
        method: methods::THREAD_ARCHIVE,
        request: RpcRequestContract::ThreadId,
        response: RpcResponseContract::Object,
    },
    RpcContractDescriptor {
        method: methods::THREAD_READ,
        request: RpcRequestContract::ThreadId,
        response: RpcResponseContract::ThreadId,
    },
    RpcContractDescriptor {
        method: methods::THREAD_LIST,
        request: RpcRequestContract::Object,
        response: RpcResponseContract::DataArray,
    },
    RpcContractDescriptor {
        method: methods::THREAD_LOADED_LIST,
        request: RpcRequestContract::Object,
        response: RpcResponseContract::DataArray,
    },
    RpcContractDescriptor {
        method: methods::THREAD_ROLLBACK,
        request: RpcRequestContract::ThreadId,
        response: RpcResponseContract::ThreadId,
    },
    RpcContractDescriptor {
        method: methods::TURN_START,
        request: RpcRequestContract::ThreadId,
        response: RpcResponseContract::TurnId,
    },
    RpcContractDescriptor {
        method: methods::TURN_INTERRUPT,
        request: RpcRequestContract::ThreadIdAndTurnId,
        response: RpcResponseContract::Object,
    },
];

/// Canonical RPC contract descriptor list (single source of truth).
pub fn rpc_contract_descriptors() -> &'static [RpcContractDescriptor] {
    &RPC_CONTRACT_DESCRIPTORS
}

/// Contract descriptor for one method, when the method is known.
pub fn rpc_contract_descriptor(method: &str) -> Option<&'static RpcContractDescriptor> {
    RPC_CONTRACT_DESCRIPTORS
        .iter()
        .find(|descriptor| descriptor.method == method)
}

/// Validate outgoing JSON-RPC request payload for one method.
///
/// - Always validates that method name is non-empty.
/// - In `KnownMethods` mode, validates request shape for known methods.
pub fn validate_rpc_request(
    method: &str,
    params: &Value,
    mode: RpcValidationMode,
) -> Result<(), RpcError> {
    validate_method_name(method)?;

    if mode == RpcValidationMode::None {
        return Ok(());
    }

    match rpc_contract_descriptor(method) {
        Some(descriptor) => validate_request_by_descriptor(method, params, *descriptor),
        None => Ok(()),
    }
}

/// Validate incoming JSON-RPC result payload for one method.
///
/// In `KnownMethods` mode this enforces minimum shape invariants for known methods.
pub fn validate_rpc_response(
    method: &str,
    result: &Value,
    mode: RpcValidationMode,
) -> Result<(), RpcError> {
    validate_method_name(method)?;

    if mode == RpcValidationMode::None {
        return Ok(());
    }

    match rpc_contract_descriptor(method) {
        Some(descriptor) => validate_response_by_descriptor(method, result, *descriptor),
        None => Ok(()),
    }
}

fn validate_request_by_descriptor(
    method: &str,
    params: &Value,
    descriptor: RpcContractDescriptor,
) -> Result<(), RpcError> {
    match descriptor.request {
        RpcRequestContract::Object => {
            require_object(params, method, "params")?;
            Ok(())
        }
        RpcRequestContract::ThreadStart => validate_thread_start_request(params, method),
        RpcRequestContract::ThreadId => require_string(params, method, "threadId", "params"),
        RpcRequestContract::ThreadIdAndTurnId => {
            require_string(params, method, "threadId", "params")?;
            require_string(params, method, "turnId", "params")
        }
    }
}

fn validate_response_by_descriptor(
    method: &str,
    result: &Value,
    descriptor: RpcContractDescriptor,
) -> Result<(), RpcError> {
    match descriptor.response {
        RpcResponseContract::Object => {
            require_object(result, method, "result")?;
            Ok(())
        }
        RpcResponseContract::ThreadId => {
            if parse_thread_id(result).is_none() {
                Err(invalid_response(
                    method,
                    "result is missing thread id",
                    result,
                ))
            } else {
                Ok(())
            }
        }
        RpcResponseContract::TurnId => {
            if parse_turn_id(result).is_none() {
                Err(invalid_response(
                    method,
                    "result is missing turn id",
                    result,
                ))
            } else {
                Ok(())
            }
        }
        RpcResponseContract::DataArray => {
            let obj = require_object(result, method, "result")?;
            match obj.get("data") {
                Some(Value::Array(_)) => Ok(()),
                _ => Err(invalid_response(
                    method,
                    "result.data must be an array",
                    result,
                )),
            }
        }
    }
}

fn validate_method_name(method: &str) -> Result<(), RpcError> {
    if method.trim().is_empty() {
        return Err(RpcError::InvalidRequest(
            "json-rpc method must not be empty".to_owned(),
        ));
    }
    Ok(())
}

fn require_object<'a>(
    value: &'a Value,
    method: &str,
    field_name: &str,
) -> Result<&'a serde_json::Map<String, Value>, RpcError> {
    value
        .as_object()
        .ok_or_else(|| invalid_request(method, &format!("{field_name} must be an object"), value))
}

fn require_string(
    value: &Value,
    method: &str,
    key: &str,
    field_name: &str,
) -> Result<(), RpcError> {
    let obj = require_object(value, method, field_name)?;
    match obj.get(key).and_then(Value::as_str) {
        Some(v) if !v.trim().is_empty() => Ok(()),
        _ => Err(invalid_request(
            method,
            &format!("{field_name}.{key} must be a non-empty string"),
            value,
        )),
    }
}

fn validate_thread_start_request(params: &Value, method: &str) -> Result<(), RpcError> {
    let obj = require_object(params, method, "params")?;

    // thread/start uses legacy "sandbox" enum string; sandboxPolicy is turn-level.
    if obj.contains_key("sandboxPolicy") {
        return Err(invalid_request(
            method,
            "params.sandboxPolicy is not valid for thread/start; use params.sandbox",
            params,
        ));
    }
    if let Some(sandbox) = obj.get("sandbox") {
        match sandbox.as_str().filter(|value| !value.trim().is_empty()) {
            Some(_) => {}
            None => {
                return Err(invalid_request(
                    method,
                    "params.sandbox must be a non-empty string when provided",
                    params,
                ));
            }
        }
    }
    Ok(())
}

fn invalid_request(method: &str, reason: &str, payload: &Value) -> RpcError {
    RpcError::InvalidRequest(format!(
        "invalid json-rpc request for {method}: {reason}; payload={payload}"
    ))
}

fn invalid_response(method: &str, reason: &str, payload: &Value) -> RpcError {
    RpcError::InvalidRequest(format!(
        "invalid json-rpc response for {method}: {reason}; payload={payload}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rejects_empty_method() {
        let err = validate_rpc_request("", &json!({}), RpcValidationMode::KnownMethods)
            .expect_err("empty method must fail");
        assert!(matches!(err, RpcError::InvalidRequest(_)));
    }

    #[test]
    fn validates_turn_interrupt_params_shape() {
        let err = validate_rpc_request(
            "turn/interrupt",
            &json!({"threadId":"thr"}),
            RpcValidationMode::KnownMethods,
        )
        .expect_err("missing turnId must fail");
        assert!(matches!(err, RpcError::InvalidRequest(_)));

        validate_rpc_request(
            "turn/interrupt",
            &json!({"threadId":"thr", "turnId":"turn"}),
            RpcValidationMode::KnownMethods,
        )
        .expect("valid params");
    }

    #[test]
    fn validates_thread_start_rejects_turn_level_sandbox_policy_key() {
        let err = validate_rpc_request(
            "thread/start",
            &json!({"cwd":"/tmp","sandboxPolicy":{"type":"readOnly"}}),
            RpcValidationMode::KnownMethods,
        )
        .expect_err("thread/start must reject sandboxPolicy key");
        assert!(matches!(err, RpcError::InvalidRequest(_)));
    }

    #[test]
    fn validates_thread_start_accepts_legacy_sandbox_string() {
        validate_rpc_request(
            "thread/start",
            &json!({"cwd":"/tmp","sandbox":"read-only"}),
            RpcValidationMode::KnownMethods,
        )
        .expect("thread/start should accept sandbox string");
    }

    #[test]
    fn validates_thread_start_response_thread_id() {
        let err = validate_rpc_response(
            "thread/start",
            &json!({"thread": {}}),
            RpcValidationMode::KnownMethods,
        )
        .expect_err("missing thread id must fail");
        assert!(matches!(err, RpcError::InvalidRequest(_)));

        validate_rpc_response(
            "thread/start",
            &json!({"thread": {"id":"thr_1"}}),
            RpcValidationMode::KnownMethods,
        )
        .expect("valid response");
    }

    #[test]
    fn validates_turn_start_response_turn_id() {
        let err = validate_rpc_response(
            "turn/start",
            &json!({"turn": {}}),
            RpcValidationMode::KnownMethods,
        )
        .expect_err("missing turn id must fail");
        assert!(matches!(err, RpcError::InvalidRequest(_)));

        validate_rpc_response(
            "turn/start",
            &json!({"turn": {"id":"turn_1"}}),
            RpcValidationMode::KnownMethods,
        )
        .expect("valid response");
    }

    #[test]
    fn passes_unknown_method_in_known_mode() {
        validate_rpc_request(
            "echo/custom",
            &json!({"k":"v"}),
            RpcValidationMode::KnownMethods,
        )
        .expect("unknown method request should pass");
        validate_rpc_response(
            "echo/custom",
            &json!({"ok":true}),
            RpcValidationMode::KnownMethods,
        )
        .expect("unknown method response should pass");
    }

    #[test]
    fn known_method_catalog_is_stable() {
        assert_eq!(
            methods::KNOWN,
            [
                methods::THREAD_START,
                methods::THREAD_RESUME,
                methods::THREAD_FORK,
                methods::THREAD_ARCHIVE,
                methods::THREAD_READ,
                methods::THREAD_LIST,
                methods::THREAD_LOADED_LIST,
                methods::THREAD_ROLLBACK,
                methods::TURN_START,
                methods::TURN_INTERRUPT,
            ]
        );
    }

    #[test]
    fn descriptor_catalog_matches_known_method_catalog() {
        let descriptor_methods: Vec<&'static str> = rpc_contract_descriptors()
            .iter()
            .map(|descriptor| descriptor.method)
            .collect();
        assert_eq!(descriptor_methods, methods::KNOWN);
    }

    #[test]
    fn default_validation_mode_is_known_methods() {
        assert_eq!(
            RpcValidationMode::default(),
            RpcValidationMode::KnownMethods
        );
    }

    #[test]
    fn skips_validation_in_none_mode() {
        validate_rpc_request("", &json!(null), RpcValidationMode::None)
            .expect_err("empty method must still fail");

        validate_rpc_request("turn/start", &json!(null), RpcValidationMode::None)
            .expect("none mode skips params shape");
        validate_rpc_response("turn/start", &json!(null), RpcValidationMode::None)
            .expect("none mode skips result shape");
    }
}
