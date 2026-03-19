use serde_json::Value;

use crate::runtime::api::summarize_sandbox_policy_wire_value;
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
    pub const SKILLS_LIST: &str = "skills/list";
    pub const COMMAND_EXEC: &str = "command/exec";
    pub const COMMAND_EXEC_WRITE: &str = "command/exec/write";
    pub const COMMAND_EXEC_TERMINATE: &str = "command/exec/terminate";
    pub const COMMAND_EXEC_RESIZE: &str = "command/exec/resize";
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
    pub const COMMAND_EXEC_OUTPUT_DELTA: &str = "command/exec/outputDelta";
    pub const ITEM_COMPLETED: &str = "item/completed";
    pub const APPROVAL_ACK: &str = "approval/ack";
    pub const SKILLS_CHANGED: &str = "skills/changed";

    pub const KNOWN: [&str; 15] = [
        THREAD_START,
        THREAD_RESUME,
        THREAD_FORK,
        THREAD_ARCHIVE,
        THREAD_READ,
        THREAD_LIST,
        THREAD_LOADED_LIST,
        THREAD_ROLLBACK,
        SKILLS_LIST,
        COMMAND_EXEC,
        COMMAND_EXEC_WRITE,
        COMMAND_EXEC_TERMINATE,
        COMMAND_EXEC_RESIZE,
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
    ProcessId,
    CommandExec,
    CommandExecWrite,
    CommandExecResize,
}

/// Response-shape rule for one RPC method contract descriptor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RpcResponseContract {
    Object,
    ThreadId,
    TurnId,
    DataArray,
    CommandExec,
}

/// Single-source descriptor for one app-server RPC contract method.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RpcContractDescriptor {
    pub method: &'static str,
    pub request: RpcRequestContract,
    pub response: RpcResponseContract,
}

const FIELD_PARAMS: &str = "params";
const FIELD_RESULT: &str = "result";
const FIELD_PARAMS_SANDBOX_POLICY: &str = "params.sandboxPolicy";
const KEY_DATA: &str = "data";
const KEY_PROCESS_ID: &str = "processId";
const KEY_SIZE: &str = "size";

const RPC_CONTRACT_DESCRIPTORS: [RpcContractDescriptor; 15] = [
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
        method: methods::SKILLS_LIST,
        request: RpcRequestContract::Object,
        response: RpcResponseContract::DataArray,
    },
    RpcContractDescriptor {
        method: methods::COMMAND_EXEC,
        request: RpcRequestContract::CommandExec,
        response: RpcResponseContract::CommandExec,
    },
    RpcContractDescriptor {
        method: methods::COMMAND_EXEC_WRITE,
        request: RpcRequestContract::CommandExecWrite,
        response: RpcResponseContract::Object,
    },
    RpcContractDescriptor {
        method: methods::COMMAND_EXEC_TERMINATE,
        request: RpcRequestContract::ProcessId,
        response: RpcResponseContract::Object,
    },
    RpcContractDescriptor {
        method: methods::COMMAND_EXEC_RESIZE,
        request: RpcRequestContract::CommandExecResize,
        response: RpcResponseContract::Object,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RpcContractSurface {
    Request,
    Response,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum RpcContractViolation {
    EmptyMethod,
    FieldMustBeObject { field_name: String },
    FieldMustBeNonEmptyString { field_name: String, key: String },
    MissingThreadId,
    MissingTurnId,
    ResultDataMustBeArray,
    CommandMustBeArray,
    CommandMustNotBeEmpty,
    CommandItemsMustBeStrings,
    ProcessIdRequiredForStreaming,
    DisableOutputCapConflictsWithOutputBytesCap,
    DisableTimeoutConflictsWithTimeoutMs,
    TimeoutMsMustBeNonNegative,
    OutputBytesCapMustBePositive,
    SizeRequiresTty,
    SizeMustBeObject,
    SizeRowsMustBePositive,
    SizeColsMustBePositive,
    WriteRequestMustIncludeDeltaOrCloseStdin,
    ExitCodeMustBeI32CompatibleInteger,
    StdoutMustBeString,
    StderrMustBeString,
    ParamsFieldMustBeString { key: String },
    Custom(String),
}

impl RpcContractViolation {
    fn reason(&self) -> String {
        match self {
            Self::EmptyMethod => "json-rpc method must not be empty".to_owned(),
            Self::FieldMustBeObject { field_name } => format!("{field_name} must be an object"),
            Self::FieldMustBeNonEmptyString { field_name, key } => {
                format!("{field_name}.{key} must be a non-empty string")
            }
            Self::MissingThreadId => "result is missing thread id".to_owned(),
            Self::MissingTurnId => "result is missing turn id".to_owned(),
            Self::ResultDataMustBeArray => "result.data must be an array".to_owned(),
            Self::CommandMustBeArray => "params.command must be an array".to_owned(),
            Self::CommandMustNotBeEmpty => "params.command must not be empty".to_owned(),
            Self::CommandItemsMustBeStrings => "params.command items must be strings".to_owned(),
            Self::ProcessIdRequiredForStreaming => {
                "params.processId is required when tty or streaming is enabled".to_owned()
            }
            Self::DisableOutputCapConflictsWithOutputBytesCap => {
                "params.disableOutputCap cannot be combined with params.outputBytesCap".to_owned()
            }
            Self::DisableTimeoutConflictsWithTimeoutMs => {
                "params.disableTimeout cannot be combined with params.timeoutMs".to_owned()
            }
            Self::TimeoutMsMustBeNonNegative => "params.timeoutMs must be >= 0".to_owned(),
            Self::OutputBytesCapMustBePositive => "params.outputBytesCap must be > 0".to_owned(),
            Self::SizeRequiresTty => "params.size is only valid when params.tty is true".to_owned(),
            Self::SizeMustBeObject => "params.size must be an object".to_owned(),
            Self::SizeRowsMustBePositive => "params.size.rows must be > 0".to_owned(),
            Self::SizeColsMustBePositive => "params.size.cols must be > 0".to_owned(),
            Self::WriteRequestMustIncludeDeltaOrCloseStdin => {
                "params must include deltaBase64, closeStdin, or both".to_owned()
            }
            Self::ExitCodeMustBeI32CompatibleInteger => {
                "result.exitCode must be an i32-compatible integer".to_owned()
            }
            Self::StdoutMustBeString => "result.stdout must be a string".to_owned(),
            Self::StderrMustBeString => "result.stderr must be a string".to_owned(),
            Self::ParamsFieldMustBeString { key } => format!("params.{key} must be a string"),
            Self::Custom(reason) => reason.clone(),
        }
    }
}

fn validate_request_by_descriptor(
    method: &str,
    params: &Value,
    descriptor: RpcContractDescriptor,
) -> Result<(), RpcError> {
    match descriptor.request {
        RpcRequestContract::Object => {
            require_object(params, method, FIELD_PARAMS)?;
            Ok(())
        }
        RpcRequestContract::ThreadStart => validate_thread_start_request(params, method),
        RpcRequestContract::ThreadId => require_string(params, method, "threadId", FIELD_PARAMS),
        RpcRequestContract::ThreadIdAndTurnId => {
            require_string(params, method, "threadId", FIELD_PARAMS)?;
            require_string(params, method, "turnId", FIELD_PARAMS)
        }
        RpcRequestContract::ProcessId => {
            require_string(params, method, KEY_PROCESS_ID, FIELD_PARAMS)
        }
        RpcRequestContract::CommandExec => validate_command_exec_request(params, method),
        RpcRequestContract::CommandExecWrite => validate_command_exec_write_request(params, method),
        RpcRequestContract::CommandExecResize => {
            validate_command_exec_resize_request(params, method)
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
            require_response_object(result, method, FIELD_RESULT)?;
            Ok(())
        }
        RpcResponseContract::ThreadId => {
            if parse_thread_id(result).is_none() {
                Err(project_contract_violation(
                    method,
                    RpcContractSurface::Response,
                    &RpcContractViolation::MissingThreadId,
                    result,
                ))
            } else {
                Ok(())
            }
        }
        RpcResponseContract::TurnId => {
            if parse_turn_id(result).is_none() {
                Err(project_contract_violation(
                    method,
                    RpcContractSurface::Response,
                    &RpcContractViolation::MissingTurnId,
                    result,
                ))
            } else {
                Ok(())
            }
        }
        RpcResponseContract::DataArray => {
            let obj = require_response_object(result, method, FIELD_RESULT)?;
            match obj.get(KEY_DATA) {
                Some(Value::Array(_)) => Ok(()),
                _ => Err(project_contract_violation(
                    method,
                    RpcContractSurface::Response,
                    &RpcContractViolation::ResultDataMustBeArray,
                    result,
                )),
            }
        }
        RpcResponseContract::CommandExec => validate_command_exec_response(result, method),
    }
}

fn validate_method_name(method: &str) -> Result<(), RpcError> {
    if method.trim().is_empty() {
        return Err(project_contract_violation(
            method,
            RpcContractSurface::Request,
            &RpcContractViolation::EmptyMethod,
            &Value::Null,
        ));
    }
    Ok(())
}

fn require_object<'a>(
    value: &'a Value,
    method: &str,
    field_name: &str,
) -> Result<&'a serde_json::Map<String, Value>, RpcError> {
    require_object_on(RpcContractSurface::Request, value, method, field_name)
}

fn require_response_object<'a>(
    value: &'a Value,
    method: &str,
    field_name: &str,
) -> Result<&'a serde_json::Map<String, Value>, RpcError> {
    require_object_on(RpcContractSurface::Response, value, method, field_name)
}

fn require_object_on<'a>(
    surface: RpcContractSurface,
    value: &'a Value,
    method: &str,
    field_name: &str,
) -> Result<&'a serde_json::Map<String, Value>, RpcError> {
    value.as_object().ok_or_else(|| {
        project_contract_violation(
            method,
            surface,
            &RpcContractViolation::FieldMustBeObject {
                field_name: field_name.to_owned(),
            },
            value,
        )
    })
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
        _ => Err(project_contract_violation(
            method,
            RpcContractSurface::Request,
            &RpcContractViolation::FieldMustBeNonEmptyString {
                field_name: field_name.to_owned(),
                key: key.to_owned(),
            },
            value,
        )),
    }
}

fn validate_thread_start_request(params: &Value, method: &str) -> Result<(), RpcError> {
    require_object(params, method, FIELD_PARAMS)?;
    Ok(())
}

fn validate_command_exec_request(params: &Value, method: &str) -> Result<(), RpcError> {
    let obj = require_object(params, method, FIELD_PARAMS)?;
    let command = obj
        .get("command")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            project_contract_violation(
                method,
                RpcContractSurface::Request,
                &RpcContractViolation::CommandMustBeArray,
                params,
            )
        })?;
    if command.is_empty() {
        return Err(project_contract_violation(
            method,
            RpcContractSurface::Request,
            &RpcContractViolation::CommandMustNotBeEmpty,
            params,
        ));
    }
    if command.iter().any(|value| value.as_str().is_none()) {
        return Err(project_contract_violation(
            method,
            RpcContractSurface::Request,
            &RpcContractViolation::CommandItemsMustBeStrings,
            params,
        ));
    }

    let process_id = get_optional_non_empty_string(obj, KEY_PROCESS_ID).map_err(|violation| {
        project_contract_violation(method, RpcContractSurface::Request, &violation, params)
    })?;
    let tty = get_bool(obj, "tty");
    let stream_stdin = get_bool(obj, "streamStdin");
    let stream_stdout_stderr = get_bool(obj, "streamStdoutStderr");
    let effective_stream_stdin = tty || stream_stdin;
    let effective_stream_stdout_stderr = tty || stream_stdout_stderr;

    if (tty || effective_stream_stdin || effective_stream_stdout_stderr) && process_id.is_none() {
        return Err(project_contract_violation(
            method,
            RpcContractSurface::Request,
            &RpcContractViolation::ProcessIdRequiredForStreaming,
            params,
        ));
    }
    if get_bool(obj, "disableOutputCap") && obj.get("outputBytesCap").is_some() {
        return Err(project_contract_violation(
            method,
            RpcContractSurface::Request,
            &RpcContractViolation::DisableOutputCapConflictsWithOutputBytesCap,
            params,
        ));
    }
    if get_bool(obj, "disableTimeout") && obj.get("timeoutMs").is_some() {
        return Err(project_contract_violation(
            method,
            RpcContractSurface::Request,
            &RpcContractViolation::DisableTimeoutConflictsWithTimeoutMs,
            params,
        ));
    }
    if let Some(timeout_ms) = obj.get("timeoutMs").and_then(Value::as_i64) {
        if timeout_ms < 0 {
            return Err(project_contract_violation(
                method,
                RpcContractSurface::Request,
                &RpcContractViolation::TimeoutMsMustBeNonNegative,
                params,
            ));
        }
    }
    if let Some(output_bytes_cap) = obj.get("outputBytesCap").and_then(Value::as_u64) {
        if output_bytes_cap == 0 {
            return Err(project_contract_violation(
                method,
                RpcContractSurface::Request,
                &RpcContractViolation::OutputBytesCapMustBePositive,
                params,
            ));
        }
    }
    if let Some(size) = obj.get(KEY_SIZE) {
        if !tty {
            return Err(project_contract_violation(
                method,
                RpcContractSurface::Request,
                &RpcContractViolation::SizeRequiresTty,
                params,
            ));
        }
        validate_command_exec_size(size, method, params)?;
    }
    if let Some(sandbox_policy) = obj.get("sandboxPolicy") {
        summarize_sandbox_policy_wire_value(sandbox_policy, FIELD_PARAMS_SANDBOX_POLICY)
            .map_err(|reason| invalid_request(method, &reason, params))?;
    }

    Ok(())
}

fn validate_command_exec_write_request(params: &Value, method: &str) -> Result<(), RpcError> {
    require_string(params, method, KEY_PROCESS_ID, FIELD_PARAMS)?;
    let obj = require_object(params, method, FIELD_PARAMS)?;
    let has_delta = obj.get("deltaBase64").and_then(Value::as_str).is_some();
    let close_stdin = get_bool(obj, "closeStdin");
    if !has_delta && !close_stdin {
        return Err(project_contract_violation(
            method,
            RpcContractSurface::Request,
            &RpcContractViolation::WriteRequestMustIncludeDeltaOrCloseStdin,
            params,
        ));
    }
    Ok(())
}

fn validate_command_exec_resize_request(params: &Value, method: &str) -> Result<(), RpcError> {
    require_string(params, method, KEY_PROCESS_ID, FIELD_PARAMS)?;
    let obj = require_object(params, method, FIELD_PARAMS)?;
    let size = obj.get(KEY_SIZE).ok_or_else(|| {
        project_contract_violation(
            method,
            RpcContractSurface::Request,
            &RpcContractViolation::SizeMustBeObject,
            params,
        )
    })?;
    validate_command_exec_size(size, method, params)
}

fn validate_command_exec_response(result: &Value, method: &str) -> Result<(), RpcError> {
    let obj = require_response_object(result, method, FIELD_RESULT)?;
    match obj.get("exitCode").and_then(Value::as_i64) {
        Some(code) if i32::try_from(code).is_ok() => {}
        _ => {
            return Err(project_contract_violation(
                method,
                RpcContractSurface::Response,
                &RpcContractViolation::ExitCodeMustBeI32CompatibleInteger,
                result,
            ));
        }
    }
    if obj.get("stdout").and_then(Value::as_str).is_none() {
        return Err(project_contract_violation(
            method,
            RpcContractSurface::Response,
            &RpcContractViolation::StdoutMustBeString,
            result,
        ));
    }
    if obj.get("stderr").and_then(Value::as_str).is_none() {
        return Err(project_contract_violation(
            method,
            RpcContractSurface::Response,
            &RpcContractViolation::StderrMustBeString,
            result,
        ));
    }
    Ok(())
}

fn validate_command_exec_size(size: &Value, method: &str, payload: &Value) -> Result<(), RpcError> {
    let size_obj = size.as_object().ok_or_else(|| {
        project_contract_violation(
            method,
            RpcContractSurface::Request,
            &RpcContractViolation::SizeMustBeObject,
            payload,
        )
    })?;
    let rows = size_obj.get("rows").and_then(Value::as_u64).unwrap_or(0);
    let cols = size_obj.get("cols").and_then(Value::as_u64).unwrap_or(0);
    if rows == 0 {
        return Err(project_contract_violation(
            method,
            RpcContractSurface::Request,
            &RpcContractViolation::SizeRowsMustBePositive,
            payload,
        ));
    }
    if cols == 0 {
        return Err(project_contract_violation(
            method,
            RpcContractSurface::Request,
            &RpcContractViolation::SizeColsMustBePositive,
            payload,
        ));
    }
    Ok(())
}

fn get_optional_non_empty_string<'a>(
    obj: &'a serde_json::Map<String, Value>,
    key: &str,
) -> Result<Option<&'a str>, RpcContractViolation> {
    match obj.get(key) {
        Some(Value::String(text)) if !text.trim().is_empty() => Ok(Some(text)),
        Some(Value::String(_)) => Err(RpcContractViolation::FieldMustBeNonEmptyString {
            field_name: FIELD_PARAMS.to_owned(),
            key: key.to_owned(),
        }),
        Some(_) => Err(RpcContractViolation::ParamsFieldMustBeString {
            key: key.to_owned(),
        }),
        None => Ok(None),
    }
}

fn get_bool(obj: &serde_json::Map<String, Value>, key: &str) -> bool {
    obj.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn invalid_request(method: &str, reason: &str, payload: &Value) -> RpcError {
    project_contract_violation(
        method,
        RpcContractSurface::Request,
        &RpcContractViolation::Custom(reason.to_owned()),
        payload,
    )
}

fn project_contract_violation(
    method: &str,
    surface: RpcContractSurface,
    violation: &RpcContractViolation,
    payload: &Value,
) -> RpcError {
    let side = match surface {
        RpcContractSurface::Request => "request",
        RpcContractSurface::Response => "response",
    };
    RpcError::InvalidRequest(format!(
        "invalid json-rpc {side} for {method}: {}; payload={}",
        violation.reason(),
        payload_summary(payload),
    ))
}

pub(crate) fn payload_summary(payload: &Value) -> String {
    const MAX_KEYS: usize = 6;
    match payload {
        Value::Object(map) => {
            let mut keys: Vec<&str> = map.keys().map(|key| key.as_str()).collect();
            keys.sort_unstable();
            let preview: Vec<&str> = keys.into_iter().take(MAX_KEYS).collect();
            let more = if map.len() > MAX_KEYS { ",..." } else { "" };
            format!("object(keys=[{}{}])", preview.join(","), more)
        }
        Value::Array(items) => format!("array(len={})", items.len()),
        Value::String(text) => format!("string(len={})", text.len()),
        Value::Number(_) => "number".to_owned(),
        Value::Bool(_) => "bool".to_owned(),
        Value::Null => "null".to_owned(),
    }
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
    fn validates_skills_list_response_shape() {
        let err = validate_rpc_response(
            "skills/list",
            &json!({"skills":[]}),
            RpcValidationMode::KnownMethods,
        )
        .expect_err("missing result.data must fail");
        assert!(matches!(err, RpcError::InvalidRequest(_)));

        validate_rpc_response(
            "skills/list",
            &json!({"data":[]}),
            RpcValidationMode::KnownMethods,
        )
        .expect("valid response");
    }

    #[test]
    fn validates_command_exec_request_constraints() {
        let err = validate_rpc_request(
            "command/exec",
            &json!({"command":["bash"],"tty":true}),
            RpcValidationMode::KnownMethods,
        )
        .expect_err("tty without processId must fail");
        assert!(matches!(err, RpcError::InvalidRequest(_)));

        let err = validate_rpc_request(
            "command/exec",
            &json!({"command":["bash"],"disableTimeout":true,"timeoutMs":1}),
            RpcValidationMode::KnownMethods,
        )
        .expect_err("disableTimeout + timeoutMs must fail");
        assert!(matches!(err, RpcError::InvalidRequest(_)));

        validate_rpc_request(
            "command/exec",
            &json!({"command":["bash"],"processId":"proc-1","tty":true}),
            RpcValidationMode::KnownMethods,
        )
        .expect("tty with processId should pass");
    }

    #[test]
    fn validates_command_exec_request_rejects_non_string_process_id() {
        let err = validate_rpc_request(
            "command/exec",
            &json!({"command":["bash"],"processId":123}),
            RpcValidationMode::KnownMethods,
        )
        .expect_err("non-string processId must fail");

        let RpcError::InvalidRequest(message) = err else {
            panic!("expected invalid request");
        };
        assert!(message.contains("params.processId must be a string"));
    }

    #[test]
    fn validates_command_exec_response_shape() {
        let err = validate_rpc_response(
            "command/exec",
            &json!({"exitCode":0,"stdout":"ok"}),
            RpcValidationMode::KnownMethods,
        )
        .expect_err("stderr missing must fail");
        assert!(matches!(err, RpcError::InvalidRequest(_)));

        validate_rpc_response(
            "command/exec",
            &json!({"exitCode":0,"stdout":"ok","stderr":""}),
            RpcValidationMode::KnownMethods,
        )
        .expect("valid command exec response");
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
                methods::SKILLS_LIST,
                methods::COMMAND_EXEC,
                methods::COMMAND_EXEC_WRITE,
                methods::COMMAND_EXEC_TERMINATE,
                methods::COMMAND_EXEC_RESIZE,
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

    #[test]
    fn invalid_request_error_redacts_payload_values() {
        let err = validate_rpc_request(
            "turn/interrupt",
            &json!({"threadId":"thr_sensitive","secret":"token-123"}),
            RpcValidationMode::KnownMethods,
        )
        .expect_err("missing turnId must fail");

        let RpcError::InvalidRequest(message) = err else {
            panic!("expected invalid request");
        };
        assert!(message.contains("invalid json-rpc request for turn/interrupt"));
        assert!(message.contains("params.turnId must be a non-empty string"));
        assert!(message.contains("payload=object(keys=[secret,threadId])"));
        assert!(!message.contains("token-123"));
        assert!(!message.contains("thr_sensitive"));
    }

    #[test]
    fn invalid_response_error_redacts_payload_values() {
        let err = validate_rpc_response(
            "thread/start",
            &json!({"thread": {}, "secret": {"token":"abc"}}),
            RpcValidationMode::KnownMethods,
        )
        .expect_err("missing thread id must fail");

        let RpcError::InvalidRequest(message) = err else {
            panic!("expected invalid request");
        };
        assert!(message.contains("invalid json-rpc response for thread/start"));
        assert!(message.contains("result is missing thread id"));
        assert!(message.contains("payload=object(keys=[secret,thread])"));
        assert!(!message.contains("abc"));
    }

    #[test]
    fn rejects_response_scalar_id_fallback() {
        let err = validate_rpc_response(
            "thread/start",
            &json!("thr_scalar"),
            RpcValidationMode::KnownMethods,
        )
        .expect_err("scalar id fallback must not be accepted");
        assert!(matches!(err, RpcError::InvalidRequest(_)));
    }
}
