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
mod tests;
