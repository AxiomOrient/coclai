use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use coclai_runtime::{
    Envelope, Runtime, RuntimeConfig, SchemaGuardConfig, ServerRequest, StdioProcessSpec,
};
use pretty_assertions::assert_eq;
use serde_json::{json, Value};
use tokio::sync::broadcast;
use tokio::time::timeout;

const RPC_TIMEOUT: Duration = Duration::from_secs(15);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const TEST_TIMEOUT: Duration = Duration::from_secs(45);

fn update_golden_enabled() -> bool {
    matches!(
        std::env::var("APP_SERVER_CONTRACT_UPDATE_GOLDEN")
            .ok()
            .as_deref(),
        Some("1") | Some("true") | Some("TRUE") | Some("yes") | Some("YES")
    )
}

fn workspace_schema_guard() -> SchemaGuardConfig {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let active = manifest_dir.join("../../SCHEMAS/app-server/active");
    SchemaGuardConfig {
        active_schema_dir: active,
    }
}

fn python_contract_process() -> StdioProcessSpec {
    let script = r#"
import json
import sys

for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    try:
        msg = json.loads(line)
    except Exception:
        continue

    method = msg.get("method")
    rpc_id = msg.get("id")

    if method is None and ("result" in msg or "error" in msg):
        if rpc_id in (901, 902, 903):
            sys.stdout.write(json.dumps({
                "method": "approval/ack",
                "params": {
                    "approvalRpcId": rpc_id,
                    "result": msg.get("result"),
                    "error": msg.get("error")
                }
            }) + "\n")
            sys.stdout.flush()
        continue

    if method == "initialize" and rpc_id is not None:
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"ready": True}}) + "\n")
        sys.stdout.flush()
        continue

    if method == "probe_command_approval":
        sys.stdout.write(json.dumps({
            "id": 901,
            "method": "item/commandExecution/requestApproval",
            "params": {
                "threadId": "thr_cmd",
                "turnId": "turn_cmd",
                "itemId": "item_cmd",
                "command": "pwd",
                "cwd": "/tmp"
            }
        }) + "\n")
        sys.stdout.flush()

    if method == "probe_file_approval":
        sys.stdout.write(json.dumps({
            "id": 902,
            "method": "item/fileChange/requestApproval",
            "params": {
                "threadId": "thr_file",
                "turnId": "turn_file",
                "itemId": "item_file",
                "changes": [
                    {"path": "contract_file.txt", "type": "add", "content": "hello\n"}
                ]
            }
        }) + "\n")
        sys.stdout.flush()

    if method == "probe_user_input":
        sys.stdout.write(json.dumps({
            "id": 903,
            "method": "item/tool/requestUserInput",
            "params": {
                "questions": [
                    {
                        "id": "choice",
                        "type": "singleSelect",
                        "label": "Select one",
                        "options": ["yes", "no"]
                    }
                ]
            }
        }) + "\n")
        sys.stdout.flush()

    if rpc_id is None:
        continue

    sys.stdout.write(json.dumps({
        "id": rpc_id,
        "result": {"echoMethod": method}
    }) + "\n")
    sys.stdout.flush()
"#;

    let mut spec = StdioProcessSpec::new("python3");
    spec.args = vec!["-u".to_owned(), "-c".to_owned(), script.to_owned()];
    spec
}

fn runtime_config() -> RuntimeConfig {
    RuntimeConfig::new(python_contract_process(), workspace_schema_guard())
}

fn golden_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../SCHEMAS/golden/events")
        .join(name)
}

fn sorted_param_keys(params: &Value) -> Value {
    let mut keys = params
        .as_object()
        .map(|obj| obj.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    keys.sort();
    Value::Array(keys.into_iter().map(Value::String).collect())
}

fn assert_or_update_golden(path: &Path, actual: &Value) {
    if update_golden_enabled() {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create golden parent");
        }
        fs::write(
            path,
            serde_json::to_string_pretty(actual).expect("serialize golden"),
        )
        .expect("write golden");
        return;
    }

    let expected_text = fs::read_to_string(path)
        .unwrap_or_else(|err| panic!("read golden {:?} failed: {err}", path));
    let expected: Value = serde_json::from_str(&expected_text)
        .unwrap_or_else(|err| panic!("invalid golden {:?}: {err}", path));
    assert_eq!(expected, *actual, "golden mismatch at {:?}", path);
}

async fn with_test_timeout(name: &str, fut: impl std::future::Future<Output = ()>) {
    timeout(TEST_TIMEOUT, fut)
        .await
        .unwrap_or_else(|_| panic!("test timeout after {:?}: {name}", TEST_TIMEOUT));
}

async fn call_raw_with_timeout(runtime: &Runtime, method: &str, params: Value) -> Value {
    timeout(RPC_TIMEOUT, runtime.call_raw(method, params))
        .await
        .unwrap_or_else(|_| panic!("rpc timeout: {method}"))
        .unwrap_or_else(|err| panic!("rpc failed for {method}: {err}"))
}

async fn wait_for_approval_ack(
    live_rx: &mut broadcast::Receiver<Envelope>,
    approval_rpc_id: u64,
) -> Value {
    loop {
        let envelope = timeout(REQUEST_TIMEOUT, live_rx.recv())
            .await
            .expect("approval ack timeout")
            .expect("live channel closed");
        if envelope.method.as_deref() == Some("approval/ack")
            && envelope.json["params"]["approvalRpcId"] == approval_rpc_id
        {
            return envelope.json["params"].clone();
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn deterministic_contract_command_approval_matches_golden() {
    with_test_timeout(
        "deterministic_contract_command_approval_matches_golden",
        async {
            let runtime = Runtime::spawn_local(runtime_config())
                .await
                .expect("spawn runtime");
            let mut live_rx = runtime.subscribe_live();
            let mut request_rx = runtime
                .take_server_request_rx()
                .await
                .expect("take request rx");

            let _ = call_raw_with_timeout(&runtime, "probe_command_approval", json!({})).await;
            let req: ServerRequest = timeout(REQUEST_TIMEOUT, request_rx.recv())
                .await
                .expect("approval request timeout")
                .expect("approval request channel closed");
            assert_eq!(req.method, "item/commandExecution/requestApproval");

            let actual = json!({
                "method": req.method,
                "paramKeys": sorted_param_keys(&req.params),
            });
            assert_or_update_golden(&golden_path("approval_command_request.json"), &actual);

            runtime
                .respond_approval_ok(&req.approval_id, json!({ "decision": "decline" }))
                .await
                .expect("respond approval");
            let ack = wait_for_approval_ack(&mut live_rx, 901).await;
            assert_eq!(ack["result"]["decision"], "decline");

            runtime.shutdown().await.expect("shutdown");
        },
    )
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn deterministic_contract_file_approval_matches_golden() {
    with_test_timeout(
        "deterministic_contract_file_approval_matches_golden",
        async {
            let runtime = Runtime::spawn_local(runtime_config())
                .await
                .expect("spawn runtime");
            let mut live_rx = runtime.subscribe_live();
            let mut request_rx = runtime
                .take_server_request_rx()
                .await
                .expect("take request rx");

            let _ = call_raw_with_timeout(&runtime, "probe_file_approval", json!({})).await;
            let req: ServerRequest = timeout(REQUEST_TIMEOUT, request_rx.recv())
                .await
                .expect("approval request timeout")
                .expect("approval request channel closed");
            assert_eq!(req.method, "item/fileChange/requestApproval");

            let actual = json!({
                "method": req.method,
                "paramKeys": sorted_param_keys(&req.params),
            });
            assert_or_update_golden(&golden_path("approval_file_request.json"), &actual);

            runtime
                .respond_approval_ok(&req.approval_id, json!({ "decision": "decline" }))
                .await
                .expect("respond approval");
            let ack = wait_for_approval_ack(&mut live_rx, 902).await;
            assert_eq!(ack["result"]["decision"], "decline");

            runtime.shutdown().await.expect("shutdown");
        },
    )
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn deterministic_contract_tool_request_user_input_matches_golden() {
    with_test_timeout(
        "deterministic_contract_tool_request_user_input_matches_golden",
        async {
            let runtime = Runtime::spawn_local(runtime_config())
                .await
                .expect("spawn runtime");
            let mut live_rx = runtime.subscribe_live();
            let mut request_rx = runtime
                .take_server_request_rx()
                .await
                .expect("take request rx");

            let _ = call_raw_with_timeout(&runtime, "probe_user_input", json!({})).await;
            let req: ServerRequest = timeout(REQUEST_TIMEOUT, request_rx.recv())
                .await
                .expect("requestUserInput timeout")
                .expect("requestUserInput channel closed");
            assert_eq!(req.method, "item/tool/requestUserInput");

            let actual = json!({
                "method": req.method,
                "paramKeys": sorted_param_keys(&req.params),
            });
            assert_or_update_golden(
                &golden_path("tool_request_user_input_request.json"),
                &actual,
            );

            runtime
                .respond_approval_ok(
                    &req.approval_id,
                    json!({
                        "answers": {
                            "choice": {
                                "answers": ["yes"]
                            }
                        }
                    }),
                )
                .await
                .expect("respond requestUserInput");
            let ack = wait_for_approval_ack(&mut live_rx, 903).await;
            assert_eq!(ack["result"]["answers"]["choice"]["answers"][0], "yes");

            runtime.shutdown().await.expect("shutdown");
        },
    )
    .await;
}
