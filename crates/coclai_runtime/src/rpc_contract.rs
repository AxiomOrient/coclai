use serde_json::Value;

use crate::errors::RpcError;
use crate::turn_output::{parse_thread_id, parse_turn_id};

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

    if is_known_method(method) {
        require_object(params, method, "params")?;
    }

    match method {
        methods::THREAD_START => validate_thread_start_request(params, method),
        methods::THREAD_RESUME
        | methods::THREAD_FORK
        | methods::THREAD_ARCHIVE
        | methods::THREAD_READ
        | methods::THREAD_ROLLBACK => require_string(params, method, "threadId", "params"),
        methods::TURN_START => require_string(params, method, "threadId", "params"),
        methods::TURN_INTERRUPT => {
            require_string(params, method, "threadId", "params")?;
            require_string(params, method, "turnId", "params")
        }
        _ => Ok(()),
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

    match method {
        methods::THREAD_START
        | methods::THREAD_RESUME
        | methods::THREAD_FORK
        | methods::THREAD_READ
        | methods::THREAD_ROLLBACK => {
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
        methods::TURN_START => {
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
        methods::THREAD_LIST | methods::THREAD_LOADED_LIST => {
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
        methods::THREAD_ARCHIVE | methods::TURN_INTERRUPT => {
            require_object(result, method, "result")?;
            Ok(())
        }
        _ => Ok(()),
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

fn is_known_method(method: &str) -> bool {
    methods::KNOWN.contains(&method)
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
    fn skips_validation_in_none_mode() {
        validate_rpc_request("", &json!(null), RpcValidationMode::None)
            .expect_err("empty method must still fail");

        validate_rpc_request("turn/start", &json!(null), RpcValidationMode::None)
            .expect("none mode skips params shape");
        validate_rpc_response("turn/start", &json!(null), RpcValidationMode::None)
            .expect("none mode skips result shape");
    }
}
