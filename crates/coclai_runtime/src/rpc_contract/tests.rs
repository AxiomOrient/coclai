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
