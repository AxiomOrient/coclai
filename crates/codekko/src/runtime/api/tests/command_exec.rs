use std::collections::BTreeMap;
use std::time::Duration;

use crate::runtime::errors::RpcError;
use crate::runtime::events::extract_command_exec_output_delta;
use tokio::time::timeout;

use super::super::*;
use super::support::spawn_mock_runtime;

#[test]
fn command_exec_params_default_to_buffered_execution() {
    let params = CommandExecParams::default();
    assert!(params.command.is_empty());
    assert_eq!(params.process_id, None);
    assert!(!params.tty);
    assert!(!params.stream_stdin);
    assert!(!params.stream_stdout_stderr);
    assert_eq!(params.output_bytes_cap, None);
    assert!(!params.disable_output_cap);
    assert!(!params.disable_timeout);
    assert_eq!(params.timeout_ms, None);
    assert_eq!(params.cwd, None);
    assert_eq!(params.env, None);
    assert_eq!(params.size, None);
    assert_eq!(params.sandbox_policy, None);
}

#[test]
fn command_exec_params_serialize_with_tty_implications() {
    let mut env = BTreeMap::new();
    env.insert("FOO".to_owned(), Some("bar".to_owned()));
    env.insert("DROP_ME".to_owned(), None);

    let wire = super::super::wire::command_exec_params_to_wire(&CommandExecParams {
        command: vec!["bash".to_owned(), "-i".to_owned()],
        process_id: Some("proc-1".to_owned()),
        tty: true,
        stream_stdin: false,
        stream_stdout_stderr: false,
        output_bytes_cap: Some(32768),
        disable_output_cap: false,
        disable_timeout: false,
        timeout_ms: Some(1000),
        cwd: Some("/repo".to_owned()),
        env: Some(env),
        size: Some(CommandExecTerminalSize {
            rows: 48,
            cols: 160,
        }),
        sandbox_policy: Some(SandboxPolicy::Preset(SandboxPreset::ReadOnly)),
    });

    assert_eq!(wire["command"][0], "bash");
    assert_eq!(wire["processId"], "proc-1");
    assert_eq!(wire["tty"], true);
    assert_eq!(wire["streamStdin"], true);
    assert_eq!(wire["streamStdoutStderr"], true);
    assert_eq!(wire["outputBytesCap"], 32768);
    assert_eq!(wire["timeoutMs"], 1000);
    assert_eq!(wire["cwd"], "/repo");
    assert_eq!(wire["env"]["FOO"], "bar");
    assert_eq!(wire["env"]["DROP_ME"], serde_json::Value::Null);
    assert_eq!(wire["size"]["rows"], 48);
    assert_eq!(wire["size"]["cols"], 160);
    assert_eq!(wire["sandboxPolicy"]["type"], "readOnly");
}

#[tokio::test(flavor = "current_thread")]
async fn runtime_command_exec_buffered_roundtrip() {
    let runtime = spawn_mock_runtime().await;

    let response = runtime
        .command_exec(CommandExecParams {
            command: vec!["echo".to_owned(), "hi".to_owned()],
            cwd: Some("/repo".to_owned()),
            ..CommandExecParams::default()
        })
        .await
        .expect("buffered command exec");

    assert_eq!(response.exit_code, 0);
    assert_eq!(response.stdout, "buffered-stdout");
    assert_eq!(response.stderr, "buffered-stderr");

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn runtime_command_exec_streaming_roundtrip_emits_output_deltas() {
    let runtime = spawn_mock_runtime().await;
    let mut live_rx = runtime.subscribe_live();
    let runtime_for_exec = runtime.clone();

    let exec_task = tokio::spawn(async move {
        runtime_for_exec
            .command_exec(CommandExecParams {
                command: vec!["bash".to_owned(), "-i".to_owned()],
                process_id: Some("proc-1".to_owned()),
                tty: true,
                output_bytes_cap: Some(32768),
                ..CommandExecParams::default()
            })
            .await
    });

    let notifications = timeout(Duration::from_secs(2), async {
        let mut collected = Vec::new();
        while collected.len() < 2 {
            let envelope = live_rx.recv().await.expect("live envelope");
            if let Some(notification) = extract_command_exec_output_delta(&envelope) {
                collected.push(notification);
            }
        }
        collected
    })
    .await
    .expect("command exec output deltas");

    assert_eq!(notifications[0].process_id, "proc-1");
    assert_eq!(notifications[0].stream, CommandExecOutputStream::Stdout);
    assert_eq!(notifications[1].stream, CommandExecOutputStream::Stderr);

    let response = exec_task
        .await
        .expect("join")
        .expect("streaming command exec");
    assert_eq!(response.exit_code, 0);
    assert_eq!(response.stdout, "");
    assert_eq!(response.stderr, "");

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn runtime_command_exec_follow_up_helpers_work() {
    let runtime = spawn_mock_runtime().await;

    runtime
        .command_exec_write(CommandExecWriteParams {
            process_id: "proc-1".to_owned(),
            delta_base64: Some("aGVsbG8=".to_owned()),
            close_stdin: false,
        })
        .await
        .expect("command exec write");

    runtime
        .command_exec_resize(CommandExecResizeParams {
            process_id: "proc-1".to_owned(),
            size: CommandExecTerminalSize {
                rows: 40,
                cols: 120,
            },
        })
        .await
        .expect("command exec resize");

    runtime
        .command_exec_terminate(CommandExecTerminateParams {
            process_id: "proc-1".to_owned(),
        })
        .await
        .expect("command exec terminate");

    runtime.shutdown().await.expect("shutdown");
}

#[test]
fn known_validation_rejects_invalid_command_exec_shapes() {
    let err = crate::runtime::rpc_contract::validate_rpc_request(
        crate::runtime::rpc_contract::methods::COMMAND_EXEC,
        &serde_json::json!({"command":[]}),
        crate::runtime::RpcValidationMode::KnownMethods,
    )
    .expect_err("empty command must fail");
    assert!(matches!(err, RpcError::InvalidRequest(_)));

    let err = crate::runtime::rpc_contract::validate_rpc_request(
        crate::runtime::rpc_contract::methods::COMMAND_EXEC,
        &serde_json::json!({"command":["bash"],"tty":true}),
        crate::runtime::RpcValidationMode::KnownMethods,
    )
    .expect_err("tty without process id must fail");
    assert!(matches!(err, RpcError::InvalidRequest(_)));

    let err = crate::runtime::rpc_contract::validate_rpc_request(
        crate::runtime::rpc_contract::methods::COMMAND_EXEC,
        &serde_json::json!({"command":["bash"],"disableTimeout":true,"timeoutMs":1}),
        crate::runtime::RpcValidationMode::KnownMethods,
    )
    .expect_err("conflicting timeout config must fail");
    assert!(matches!(err, RpcError::InvalidRequest(_)));

    let err = crate::runtime::rpc_contract::validate_rpc_request(
        crate::runtime::rpc_contract::methods::COMMAND_EXEC_WRITE,
        &serde_json::json!({"processId":"proc-1"}),
        crate::runtime::RpcValidationMode::KnownMethods,
    )
    .expect_err("write without delta or close must fail");
    assert!(matches!(err, RpcError::InvalidRequest(_)));

    let err = crate::runtime::rpc_contract::validate_rpc_request(
        crate::runtime::rpc_contract::methods::COMMAND_EXEC_RESIZE,
        &serde_json::json!({"processId":"proc-1","size":{"rows":0,"cols":10}}),
        crate::runtime::RpcValidationMode::KnownMethods,
    )
    .expect_err("zero rows must fail");
    assert!(matches!(err, RpcError::InvalidRequest(_)));
}
