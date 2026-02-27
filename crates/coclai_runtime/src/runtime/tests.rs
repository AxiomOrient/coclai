use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::time::{sleep, timeout};

use super::*;
use crate::errors::SinkError;
use crate::events::MsgKind;
use crate::sink::EventSink;

fn python_mock_process() -> StdioProcessSpec {
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

    if rpc_id is None:
        continue

    if method is None and ("result" in msg or "error" in msg):
        if rpc_id in (777, 778, 779, 780, 781, 782, "req_str_1"):
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

    if method == "initialize":
        out = {"id": rpc_id, "result": {"ready": True}}
        sys.stdout.write(json.dumps(out) + "\n")
        sys.stdout.flush()
        continue

    if method == "probe":
        sys.stdout.write(json.dumps({
            "method": "turn/started",
            "params": {"threadId":"thr_1", "turnId":"turn_1"}
        }) + "\n")
        sys.stdout.write(json.dumps({
            "id": 777,
            "method": "item/fileChange/requestApproval",
            "params": {"threadId":"thr_1", "turnId":"turn_1", "itemId":"item_1"}
        }) + "\n")
        sys.stdout.write("not-json\n")
        sys.stdout.write(json.dumps({"foo": "bar"}) + "\n")
        sys.stdout.flush()

    if method == "probe_unknown":
        sys.stdout.write(json.dumps({
            "id": 778,
            "method": "item/unknown/requestApproval",
            "params": {"threadId":"thr_1", "turnId":"turn_1", "itemId":"item_1"}
        }) + "\n")
        sys.stdout.flush()

    if method == "probe_timeout":
        sys.stdout.write(json.dumps({
            "id": 779,
            "method": "item/fileChange/requestApproval",
            "params": {"threadId":"thr_1", "turnId":"turn_1", "itemId":"item_1"}
        }) + "\n")
        sys.stdout.flush()

    if method == "probe_user_input":
        sys.stdout.write(json.dumps({
            "id": 780,
            "method": "item/tool/requestUserInput",
            "params": {
                "questions": [
                    {"id":"q1","type":"text","label":"name"}
                ]
            }
        }) + "\n")
        sys.stdout.flush()

    if method == "probe_dynamic_tool_call":
        sys.stdout.write(json.dumps({
            "id": 781,
            "method": "item/tool/call",
            "params": {
                "toolCallId": "tc_1",
                "title": "mock_tool",
                "input": {"k": "v"}
            }
        }) + "\n")
        sys.stdout.flush()

    if method == "probe_auth_refresh":
        sys.stdout.write(json.dumps({
            "id": 782,
            "method": "account/chatgptAuthTokens/refresh",
            "params": {
                "refreshToken": "rt_mock"
            }
        }) + "\n")
        sys.stdout.flush()

    if method == "probe_string_id":
        sys.stdout.write(json.dumps({
            "id": "req_str_1",
            "method": "item/fileChange/requestApproval",
            "params": {"threadId":"thr_1", "turnId":"turn_1", "itemId":"item_1"}
        }) + "\n")
        sys.stdout.flush()

    if method == "probe_state":
        sys.stdout.write(json.dumps({
            "method": "thread/started",
            "params": {"threadId":"thr_state"}
        }) + "\n")
        sys.stdout.write(json.dumps({
            "method": "turn/started",
            "params": {"threadId":"thr_state", "turnId":"turn_state"}
        }) + "\n")
        sys.stdout.write(json.dumps({
            "method": "item/started",
            "params": {"threadId":"thr_state", "turnId":"turn_state", "itemId":"item_state", "itemType":"agentMessage"}
        }) + "\n")
        sys.stdout.write(json.dumps({
            "method": "item/agentMessage/delta",
            "params": {"threadId":"thr_state", "turnId":"turn_state", "itemId":"item_state", "delta":"hello"}
        }) + "\n")
        sys.stdout.write(json.dumps({
            "method": "item/completed",
            "params": {"threadId":"thr_state", "turnId":"turn_state", "itemId":"item_state", "status":"completed"}
        }) + "\n")
        sys.stdout.write(json.dumps({
            "method": "turn/completed",
            "params": {"threadId":"thr_state", "turnId":"turn_state"}
        }) + "\n")
        sys.stdout.flush()

    out = {
        "id": rpc_id,
        "result": {"echoMethod": method, "params": msg.get("params")}
    }
    sys.stdout.write(json.dumps(out) + "\n")
    sys.stdout.flush()
"#;

    let mut spec = StdioProcessSpec::new("python3");
    spec.args = vec!["-u".to_owned(), "-c".to_owned(), script.to_owned()];
    spec
}

fn python_restartable_process() -> StdioProcessSpec {
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

    if method == "initialize" and rpc_id is not None:
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"ready": True}}) + "\n")
        sys.stdout.flush()
        continue

    if method == "crash_now":
        sys.exit(42)

    if rpc_id is None:
        continue

    sys.stdout.write(json.dumps({
        "id": rpc_id,
        "result": {"echoMethod": method, "params": msg.get("params")}
    }) + "\n")
    sys.stdout.flush()
"#;

    let mut spec = StdioProcessSpec::new("python3");
    spec.args = vec!["-u".to_owned(), "-c".to_owned(), script.to_owned()];
    spec
}

fn python_exit_on_initialized_process() -> StdioProcessSpec {
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

    if method == "initialize" and rpc_id is not None:
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"ready": True}}) + "\n")
        sys.stdout.flush()
        continue

    if method == "initialized":
        sys.exit(17)

    if rpc_id is None:
        continue

    sys.stdout.write(json.dumps({"id": rpc_id, "result": {"echoMethod": method}}) + "\n")
    sys.stdout.flush()
"#;

    let mut spec = StdioProcessSpec::new("python3");
    spec.args = vec!["-u".to_owned(), "-c".to_owned(), script.to_owned()];
    spec
}

fn python_hold_and_crash_process() -> StdioProcessSpec {
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

    if method == "initialize" and rpc_id is not None:
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"ready": True}}) + "\n")
        sys.stdout.flush()
        continue

    if method == "hold" and rpc_id is not None:
        continue

    if method == "crash_now":
        sys.exit(23)

    if rpc_id is None:
        continue

    sys.stdout.write(json.dumps({"id": rpc_id, "result": {"echoMethod": method}}) + "\n")
    sys.stdout.flush()
"#;

    let mut spec = StdioProcessSpec::new("python3");
    spec.args = vec!["-u".to_owned(), "-c".to_owned(), script.to_owned()];
    spec
}

fn python_initialize_error_process() -> StdioProcessSpec {
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

    if method == "initialize" and rpc_id is not None:
        sys.stdout.write(json.dumps({
            "id": rpc_id,
            "error": {"code": -32600, "message": "Invalid request: missing field `version`"}
        }) + "\n")
        sys.stdout.flush()
        continue
"#;

    let mut spec = StdioProcessSpec::new("python3");
    spec.args = vec!["-u".to_owned(), "-c".to_owned(), script.to_owned()];
    spec
}

#[derive(Debug)]
struct TempSchemaFixture {
    temp_root: PathBuf,
    active_dir: PathBuf,
}

impl TempSchemaFixture {
    fn guard(&self) -> SchemaGuardConfig {
        SchemaGuardConfig {
            active_schema_dir: self.active_dir.clone(),
        }
    }
}

impl Drop for TempSchemaFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.temp_root);
    }
}

fn workspace_schema_guard() -> SchemaGuardConfig {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let active = manifest_dir.join("../../SCHEMAS/app-server/active");
    SchemaGuardConfig {
        active_schema_dir: active,
    }
}

fn make_temp_schema_fixture(
    metadata: &str,
    files: &[(&str, &[u8])],
    manifest_override: Option<&str>,
) -> TempSchemaFixture {
    let temp_root = std::env::temp_dir().join(format!("coclai_runtime_schema_{}", Uuid::new_v4()));
    let active_dir = temp_root.join("active");
    let schema_dir = active_dir.join("json-schema");
    fs::create_dir_all(&schema_dir).expect("create schema dir");

    for (rel_path, bytes) in files {
        let full = schema_dir.join(rel_path);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).expect("create schema subdir");
        }
        fs::write(full, bytes).expect("write schema file");
    }

    let manifest = manifest_override
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| build_manifest_lines(files));
    fs::write(active_dir.join("manifest.sha256"), manifest).expect("write manifest");
    fs::write(active_dir.join("metadata.json"), metadata).expect("write metadata");

    TempSchemaFixture {
        temp_root,
        active_dir,
    }
}

fn build_manifest_lines(files: &[(&str, &[u8])]) -> String {
    let mut entries: Vec<(String, String)> = files
        .iter()
        .map(|(path, bytes)| {
            let normalized = normalize_rel_path(path);
            let mut hasher = Sha256::new();
            hasher.update(bytes);
            (normalized, hex::encode(hasher.finalize()))
        })
        .collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));

    entries
        .iter()
        .map(|(path, digest)| format!("{digest}  ./{path}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn normalize_rel_path(path: &str) -> String {
    Path::new(path).to_string_lossy().replace('\\', "/")
}

#[derive(Debug)]
struct FailAfterSink {
    fail_after: usize,
    seen: AtomicUsize,
    failures: AtomicUsize,
}

impl FailAfterSink {
    fn new(fail_after: usize) -> Self {
        Self {
            fail_after,
            seen: AtomicUsize::new(0),
            failures: AtomicUsize::new(0),
        }
    }

    fn seen(&self) -> usize {
        self.seen.load(AtomicOrdering::Relaxed)
    }

    fn failures(&self) -> usize {
        self.failures.load(AtomicOrdering::Relaxed)
    }
}

impl EventSink for FailAfterSink {
    fn on_envelope<'a>(&'a self, _envelope: &'a Envelope) -> crate::sink::EventSinkFuture<'a> {
        Box::pin(async move {
            let seen = self.seen.fetch_add(1, AtomicOrdering::Relaxed);
            if seen >= self.fail_after {
                self.failures.fetch_add(1, AtomicOrdering::Relaxed);
                return Err(SinkError::Internal("injected sink failure".to_owned()));
            }
            Ok(())
        })
    }
}

async fn spawn_mock_runtime() -> Runtime {
    let cfg = RuntimeConfig::new(python_mock_process(), workspace_schema_guard());
    Runtime::spawn_local(cfg).await.expect("runtime spawn")
}

async fn spawn_mock_runtime_with_sink(
    sink: Arc<dyn EventSink>,
    event_sink_channel_capacity: usize,
) -> Runtime {
    let mut cfg = RuntimeConfig::new(python_mock_process(), workspace_schema_guard());
    cfg.event_sink = Some(sink);
    cfg.event_sink_channel_capacity = event_sink_channel_capacity;
    Runtime::spawn_local(cfg).await.expect("runtime spawn")
}

async fn spawn_mock_runtime_with_server_cfg(server_requests: ServerRequestConfig) -> Runtime {
    let mut cfg = RuntimeConfig::new(python_mock_process(), workspace_schema_guard());
    cfg.server_requests = server_requests;
    Runtime::spawn_local(cfg).await.expect("runtime spawn")
}

async fn spawn_runtime_with_supervisor(
    process: StdioProcessSpec,
    restart: RestartPolicy,
) -> Runtime {
    let mut cfg = RuntimeConfig::new(process, workspace_schema_guard());
    cfg.supervisor = SupervisorConfig {
        restart,
        shutdown_flush_timeout_ms: 200,
        shutdown_terminate_grace_ms: 200,
    };
    Runtime::spawn_local(cfg).await.expect("runtime spawn")
}

async fn wait_for_recovery(runtime: &Runtime) -> Value {
    timeout(Duration::from_secs(3), async {
        loop {
            match runtime
                .call_raw("echo/recovered", json!({"phase":"post-crash"}))
                .await
            {
                Ok(value) => return value,
                Err(_) => sleep(Duration::from_millis(20)).await,
            }
        }
    })
    .await
    .expect("recovery timeout")
}

#[test]
fn restart_delay_stays_within_base_and_jitter_bounds() {
    for attempt in 0..8 {
        let delay = supervisor::compute_restart_delay(attempt, 10, 160);
        let delay_ms = delay.as_millis() as u64;
        let base_ms = (10u64.saturating_mul(1u64 << attempt)).min(160);
        let jitter_cap_ms = (base_ms / 10).min(1_000);

        assert!(delay_ms >= base_ms);
        assert!(delay_ms <= base_ms + jitter_cap_ms);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn spawn_auto_initializes_runtime() {
    let runtime = spawn_mock_runtime().await;
    assert!(runtime.is_initialized());

    let value = runtime
        .call_raw("echo/test", json!({"k":"v"}))
        .await
        .expect("call");
    assert_eq!(value["echoMethod"], "echo/test");
    assert_eq!(value["params"]["k"], "v");

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn spawn_fails_fast_on_initialize_error_without_hanging() {
    let cfg = RuntimeConfig::new(python_initialize_error_process(), workspace_schema_guard());
    let result = timeout(Duration::from_secs(3), Runtime::spawn_local(cfg))
        .await
        .expect("spawn_local must not hang");

    let err = match result {
        Ok(_) => panic!("spawn_local must fail on initialize error"),
        Err(err) => err,
    };
    match err {
        RuntimeError::Internal(message) => {
            assert!(message.contains("initialize handshake failed"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[tokio::test(flavor = "current_thread")]
async fn spawn_rejects_zero_channel_capacities() {
    let mut cfg = RuntimeConfig::new(python_mock_process(), workspace_schema_guard());
    cfg.live_channel_capacity = 0;
    let err = match Runtime::spawn_local(cfg).await {
        Ok(_) => panic!("must reject zero live channel capacity"),
        Err(err) => err,
    };
    assert!(matches!(err, RuntimeError::InvalidConfig(_)));

    let mut cfg = RuntimeConfig::new(python_mock_process(), workspace_schema_guard());
    cfg.server_request_channel_capacity = 0;
    let err = match Runtime::spawn_local(cfg).await {
        Ok(_) => panic!("must reject zero server-request channel capacity"),
        Err(err) => err,
    };
    assert!(matches!(err, RuntimeError::InvalidConfig(_)));

    let mut cfg = RuntimeConfig::new(python_mock_process(), workspace_schema_guard());
    cfg.event_sink = Some(Arc::new(FailAfterSink::new(0)));
    cfg.event_sink_channel_capacity = 0;
    let err = match Runtime::spawn_local(cfg).await {
        Ok(_) => panic!("must reject zero event sink channel capacity"),
        Err(err) => err,
    };
    assert!(matches!(err, RuntimeError::InvalidConfig(_)));

    let mut cfg = RuntimeConfig::new(python_mock_process(), workspace_schema_guard());
    cfg.state_projection_limits.max_threads = 0;
    let err = match Runtime::spawn_local(cfg).await {
        Ok(_) => panic!("must reject zero state projection thread cap"),
        Err(err) => err,
    };
    assert!(matches!(err, RuntimeError::InvalidConfig(_)));

    let mut cfg = RuntimeConfig::new(python_mock_process(), workspace_schema_guard());
    cfg.rpc_response_timeout = Duration::ZERO;
    let err = match Runtime::spawn_local(cfg).await {
        Ok(_) => panic!("must reject zero rpc response timeout"),
        Err(err) => err,
    };
    assert!(matches!(err, RuntimeError::InvalidConfig(_)));
}

#[tokio::test(flavor = "current_thread")]
async fn matches_10k_request_response_pairs() {
    let runtime = spawn_mock_runtime().await;

    for i in 0..10_000u64 {
        let value = runtime
            .call_raw("echo/loop", json!({"index": i}))
            .await
            .expect("call");
        assert_eq!(value["echoMethod"], "echo/loop");
        assert_eq!(value["params"]["index"], i);
    }

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_restarts_after_forced_exit() {
    let runtime = spawn_runtime_with_supervisor(
        python_restartable_process(),
        RestartPolicy::OnCrash {
            max_restarts: 3,
            base_backoff_ms: 10,
            max_backoff_ms: 40,
        },
    )
    .await;

    let crash = runtime.call_raw("crash_now", json!({})).await;
    assert!(matches!(crash, Err(RpcError::TransportClosed)));

    let recovered = wait_for_recovery(&runtime).await;
    assert_eq!(recovered["echoMethod"], "echo/recovered");

    let snapshot = runtime.state_snapshot();
    match &snapshot.connection {
        ConnectionState::Running { generation } => assert!(*generation >= 1),
        other => panic!("unexpected connection state after recovery: {other:?}"),
    }

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_transitions_dead_after_restart_limit_exceeded() {
    let runtime = spawn_runtime_with_supervisor(
        python_exit_on_initialized_process(),
        RestartPolicy::OnCrash {
            max_restarts: 1,
            base_backoff_ms: 10,
            max_backoff_ms: 20,
        },
    )
    .await;

    timeout(Duration::from_secs(3), async {
        loop {
            if runtime.state_snapshot().connection == ConnectionState::Dead {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("dead transition timeout");

    let err = runtime
        .call_raw("echo/dead", json!({}))
        .await
        .expect_err("must fail when dead");
    assert!(matches!(err, RpcError::InvalidRequest(_)));

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn pending_calls_resolve_transport_closed_on_child_exit() {
    let runtime =
        spawn_runtime_with_supervisor(python_hold_and_crash_process(), RestartPolicy::Never).await;

    let runtime_a = runtime.clone();
    let pending_a = tokio::spawn(async move { runtime_a.call_raw("hold", json!({"n":1})).await });

    let runtime_b = runtime.clone();
    let pending_b = tokio::spawn(async move { runtime_b.call_raw("hold", json!({"n":2})).await });

    sleep(Duration::from_millis(50)).await;
    let _ = runtime.notify_raw("crash_now", json!({})).await;

    let result_a = timeout(Duration::from_secs(2), pending_a)
        .await
        .expect("pending_a timeout")
        .expect("pending_a join");
    let result_b = timeout(Duration::from_secs(2), pending_b)
        .await
        .expect("pending_b timeout")
        .expect("pending_b join");

    assert!(matches!(result_a, Err(RpcError::TransportClosed)));
    assert!(matches!(result_b, Err(RpcError::TransportClosed)));

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn call_raw_returns_timeout_when_response_missing() {
    let mut cfg = RuntimeConfig::new(python_hold_and_crash_process(), workspace_schema_guard());
    cfg.rpc_response_timeout = Duration::from_millis(120);
    let runtime = Runtime::spawn_local(cfg).await.expect("runtime spawn");

    let started = Instant::now();
    let err = runtime
        .call_raw("hold", json!({"n":1}))
        .await
        .expect_err("hold call must timeout");
    assert!(matches!(err, RpcError::Timeout));
    assert!(
        started.elapsed() < Duration::from_millis(500),
        "rpc timeout exceeded expected bound: {:?}",
        started.elapsed()
    );

    let metrics = runtime.metrics_snapshot();
    assert_eq!(metrics.pending_rpc_count, 0);

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn call_validated_rejects_invalid_known_method_params() {
    let runtime = spawn_mock_runtime().await;

    let err = runtime
        .call_validated("turn/interrupt", json!({"threadId":"thr_only"}))
        .await
        .expect_err("missing turnId must fail");
    assert!(matches!(err, RpcError::InvalidRequest(_)));

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn call_validated_rejects_invalid_known_method_response_shape() {
    let runtime = spawn_mock_runtime().await;

    let err = runtime
        .call_validated("thread/start", json!({}))
        .await
        .expect_err("mock response does not include thread id");
    assert!(matches!(err, RpcError::InvalidRequest(_)));

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn notify_validated_rejects_invalid_known_method_params() {
    let runtime = spawn_mock_runtime().await;

    let err = runtime
        .notify_validated("turn/interrupt", json!({"threadId":"thr_only"}))
        .await
        .expect_err("missing turnId must fail");
    assert!(matches!(err, RuntimeError::InvalidConfig(_)));

    runtime.shutdown().await.expect("shutdown");
}

#[derive(Debug, Serialize)]
struct TurnInterruptNotifyMissingTurnId {
    #[serde(rename = "threadId")]
    thread_id: String,
}

#[derive(Debug, Serialize)]
struct TurnInterruptNotifyParams {
    #[serde(rename = "threadId")]
    thread_id: String,
    #[serde(rename = "turnId")]
    turn_id: String,
}

#[tokio::test(flavor = "current_thread")]
async fn notify_typed_validated_rejects_invalid_known_method_params() {
    let runtime = spawn_mock_runtime().await;

    let err = runtime
        .notify_typed_validated(
            "turn/interrupt",
            TurnInterruptNotifyMissingTurnId {
                thread_id: "thr_only".to_owned(),
            },
        )
        .await
        .expect_err("missing turnId must fail");
    assert!(matches!(err, RuntimeError::InvalidConfig(_)));

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn notify_typed_validated_accepts_valid_known_method_params() {
    let runtime = spawn_mock_runtime().await;

    runtime
        .notify_typed_validated(
            "turn/interrupt",
            TurnInterruptNotifyParams {
                thread_id: "thr_1".to_owned(),
                turn_id: "turn_1".to_owned(),
            },
        )
        .await
        .expect("valid turn/interrupt payload");

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn routes_server_request_notification_and_unknown() {
    let runtime = spawn_mock_runtime().await;
    let mut live_rx = runtime.subscribe_live();
    let mut server_request_rx = runtime
        .take_server_request_rx()
        .await
        .expect("take server request rx");

    let value = runtime.call_raw("probe", json!({})).await.expect("probe");
    assert_eq!(value["echoMethod"], "probe");

    let req = timeout(Duration::from_secs(2), server_request_rx.recv())
        .await
        .expect("server request timeout")
        .expect("server request closed");
    assert_eq!(req.method, "item/fileChange/requestApproval");
    assert!(!req.approval_id.is_empty());
    assert_eq!(req.params["itemId"], "item_1");

    let mut saw_notification = false;
    let mut saw_unknown = false;
    let mut saw_response = false;
    for _ in 0..8 {
        let envelope = timeout(Duration::from_secs(2), live_rx.recv())
            .await
            .expect("live timeout")
            .expect("live closed");
        if envelope.kind == MsgKind::Notification
            && envelope.method.as_deref() == Some("turn/started")
        {
            saw_notification = true;
        }
        if envelope.kind == MsgKind::Unknown {
            saw_unknown = true;
        }
        if envelope.kind == MsgKind::Response {
            saw_response = true;
        }
        if saw_notification && saw_unknown && saw_response {
            break;
        }
    }

    assert!(saw_notification);
    assert!(saw_unknown);
    assert!(saw_response);

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn take_server_request_rx_is_single_consumer() {
    let runtime = spawn_mock_runtime().await;
    let _first = runtime
        .take_server_request_rx()
        .await
        .expect("first take server request rx");

    let err = runtime
        .take_server_request_rx()
        .await
        .expect_err("second take server request rx must fail");
    assert_eq!(err, RuntimeError::ServerRequestReceiverTaken);

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn approval_response_roundtrip_ok() {
    let runtime = spawn_mock_runtime().await;
    let mut live_rx = runtime.subscribe_live();
    let mut server_request_rx = runtime
        .take_server_request_rx()
        .await
        .expect("take server request rx");

    runtime.call_raw("probe", json!({})).await.expect("probe");
    let req = timeout(Duration::from_secs(2), server_request_rx.recv())
        .await
        .expect("server request timeout")
        .expect("server request closed");

    runtime
        .respond_approval_ok(&req.approval_id, json!({"decision":"accept"}))
        .await
        .expect("respond approval");

    let mut saw_ack = false;
    for _ in 0..8 {
        let envelope = timeout(Duration::from_secs(2), live_rx.recv())
            .await
            .expect("live timeout")
            .expect("live closed");
        if envelope.kind == MsgKind::Notification
            && envelope.method.as_deref() == Some("approval/ack")
            && envelope.json["params"]["approvalRpcId"] == 777
        {
            assert_eq!(envelope.json["params"]["result"]["decision"], "accept");
            saw_ack = true;
            break;
        }
    }

    assert!(saw_ack);
    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn sink_failure_does_not_block_approval_pending_or_live_stream() {
    let sink_impl = Arc::new(FailAfterSink::new(0));
    let sink: Arc<dyn EventSink> = sink_impl.clone();
    let runtime = spawn_mock_runtime_with_sink(sink, 16).await;

    let mut live_rx = runtime.subscribe_live();
    let mut server_request_rx = runtime
        .take_server_request_rx()
        .await
        .expect("take server request rx");

    runtime.call_raw("probe", json!({})).await.expect("probe");
    let req = timeout(Duration::from_secs(2), server_request_rx.recv())
        .await
        .expect("server request timeout")
        .expect("server request closed");

    runtime
        .respond_approval_ok(&req.approval_id, json!({"decision":"accept"}))
        .await
        .expect("respond approval");

    let mut saw_ack = false;
    for _ in 0..12 {
        let envelope = timeout(Duration::from_secs(2), live_rx.recv())
            .await
            .expect("live timeout")
            .expect("live closed");
        if envelope.kind == MsgKind::Notification
            && envelope.method.as_deref() == Some("approval/ack")
            && envelope.json["params"]["approvalRpcId"] == 777
        {
            saw_ack = true;
            break;
        }
    }
    assert!(saw_ack, "live stream must continue even when sink fails");

    timeout(Duration::from_secs(2), async {
        loop {
            if sink_impl.failures() > 0 {
                break;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("sink failure not observed");

    let value = runtime
        .call_raw("echo/after_sink_failure", json!({"ok":true}))
        .await
        .expect("pending rpc path must continue");
    assert_eq!(value["echoMethod"], "echo/after_sink_failure");
    assert!(sink_impl.seen() >= 1);
    let metrics = runtime.metrics_snapshot();
    assert!(metrics.sink_write_count >= 1);
    assert!(metrics.sink_write_error_count >= 1);
    assert_eq!(metrics.pending_rpc_count, 0);
    assert_eq!(metrics.pending_server_request_count, 0);

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn metrics_snapshot_tracks_pending_and_broadcast_drop() {
    let runtime = spawn_mock_runtime().await;
    let mut server_request_rx = runtime
        .take_server_request_rx()
        .await
        .expect("take server request rx");

    runtime.call_raw("probe", json!({})).await.expect("probe");
    let req = timeout(Duration::from_secs(2), server_request_rx.recv())
        .await
        .expect("server request timeout")
        .expect("server request closed");
    runtime
        .respond_approval_ok(&req.approval_id, json!({"decision":"accept"}))
        .await
        .expect("respond approval");

    let metrics = runtime.metrics_snapshot();
    assert!(metrics.ingress_total >= 1);
    assert_eq!(metrics.pending_rpc_count, 0);
    assert_eq!(metrics.pending_server_request_count, 0);
    assert!(
        metrics.broadcast_send_failed >= 1,
        "no live subscribers should count as broadcast send failure"
    );

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn approval_payload_validation_failure_then_success() {
    let runtime = spawn_mock_runtime().await;
    let mut live_rx = runtime.subscribe_live();
    let mut server_request_rx = runtime
        .take_server_request_rx()
        .await
        .expect("take server request rx");

    runtime.call_raw("probe", json!({})).await.expect("probe");
    let req = timeout(Duration::from_secs(2), server_request_rx.recv())
        .await
        .expect("server request timeout")
        .expect("server request closed");

    let invalid = runtime
        .respond_approval_ok(&req.approval_id, json!({"unexpected":true}))
        .await;
    assert!(invalid.is_err(), "invalid payload must fail");

    runtime
        .respond_approval_ok(&req.approval_id, json!({"decision":"accept"}))
        .await
        .expect("respond approval");

    let mut saw_ack = false;
    for _ in 0..8 {
        let envelope = timeout(Duration::from_secs(2), live_rx.recv())
            .await
            .expect("live timeout")
            .expect("live closed");
        if envelope.kind == MsgKind::Notification
            && envelope.method.as_deref() == Some("approval/ack")
            && envelope.json["params"]["approvalRpcId"] == 777
        {
            saw_ack = true;
            break;
        }
    }
    assert!(saw_ack);

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn tool_request_user_input_roundtrip() {
    let runtime = spawn_mock_runtime().await;
    let mut live_rx = runtime.subscribe_live();
    let mut server_request_rx = runtime
        .take_server_request_rx()
        .await
        .expect("take server request rx");

    runtime
        .call_raw("probe_user_input", json!({}))
        .await
        .expect("probe_user_input");
    let req = timeout(Duration::from_secs(2), server_request_rx.recv())
        .await
        .expect("server request timeout")
        .expect("server request closed");
    assert_eq!(req.method, "item/tool/requestUserInput");

    runtime
        .respond_approval_ok(
            &req.approval_id,
            json!({
                "answers": {
                    "q1": {
                        "answers": ["alice"]
                    }
                }
            }),
        )
        .await
        .expect("respond user input");

    let mut saw_ack = false;
    for _ in 0..8 {
        let envelope = timeout(Duration::from_secs(2), live_rx.recv())
            .await
            .expect("live timeout")
            .expect("live closed");
        if envelope.kind == MsgKind::Notification
            && envelope.method.as_deref() == Some("approval/ack")
            && envelope.json["params"]["approvalRpcId"] == 780
        {
            assert_eq!(
                envelope.json["params"]["result"]["answers"]["q1"]["answers"][0],
                "alice"
            );
            saw_ack = true;
            break;
        }
    }
    assert!(saw_ack);

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn dynamic_tool_call_roundtrip() {
    let runtime = spawn_mock_runtime().await;
    let mut live_rx = runtime.subscribe_live();
    let mut server_request_rx = runtime
        .take_server_request_rx()
        .await
        .expect("take server request rx");

    runtime
        .call_raw("probe_dynamic_tool_call", json!({}))
        .await
        .expect("probe_dynamic_tool_call");
    let req = timeout(Duration::from_secs(2), server_request_rx.recv())
        .await
        .expect("server request timeout")
        .expect("server request closed");
    assert_eq!(req.method, "item/tool/call");

    runtime
        .respond_approval_ok(
            &req.approval_id,
            json!({
                "success": true,
                "contentItems": [{"type":"inputText","text":"done"}]
            }),
        )
        .await
        .expect("respond tool call");

    let mut saw_ack = false;
    for _ in 0..8 {
        let envelope = timeout(Duration::from_secs(2), live_rx.recv())
            .await
            .expect("live timeout")
            .expect("live closed");
        if envelope.kind == MsgKind::Notification
            && envelope.method.as_deref() == Some("approval/ack")
            && envelope.json["params"]["approvalRpcId"] == 781
        {
            assert_eq!(envelope.json["params"]["result"]["success"], true);
            assert_eq!(
                envelope.json["params"]["result"]["contentItems"][0]["text"],
                "done"
            );
            saw_ack = true;
            break;
        }
    }
    assert!(saw_ack);

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn auth_refresh_roundtrip() {
    let runtime = spawn_mock_runtime().await;
    let mut live_rx = runtime.subscribe_live();
    let mut server_request_rx = runtime
        .take_server_request_rx()
        .await
        .expect("take server request rx");

    runtime
        .call_raw("probe_auth_refresh", json!({}))
        .await
        .expect("probe_auth_refresh");
    let req = timeout(Duration::from_secs(2), server_request_rx.recv())
        .await
        .expect("server request timeout")
        .expect("server request closed");
    assert_eq!(req.method, "account/chatgptAuthTokens/refresh");

    runtime
        .respond_approval_ok(
            &req.approval_id,
            json!({
                "accessToken": "at_mock",
                "chatgptAccountId": "acct_1",
                "chatgptPlanType": null
            }),
        )
        .await
        .expect("respond auth refresh");

    let mut saw_ack = false;
    for _ in 0..8 {
        let envelope = timeout(Duration::from_secs(2), live_rx.recv())
            .await
            .expect("live timeout")
            .expect("live closed");
        if envelope.kind == MsgKind::Notification
            && envelope.method.as_deref() == Some("approval/ack")
            && envelope.json["params"]["approvalRpcId"] == 782
        {
            assert_eq!(envelope.json["params"]["result"]["accessToken"], "at_mock");
            assert_eq!(
                envelope.json["params"]["result"]["chatgptAccountId"],
                "acct_1"
            );
            saw_ack = true;
            break;
        }
    }
    assert!(saw_ack);

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn tool_request_user_input_payload_validation_rejects_missing_answers() {
    let runtime = spawn_mock_runtime().await;
    let mut server_request_rx = runtime
        .take_server_request_rx()
        .await
        .expect("take server request rx");

    runtime
        .call_raw("probe_user_input", json!({}))
        .await
        .expect("probe_user_input");
    let req = timeout(Duration::from_secs(2), server_request_rx.recv())
        .await
        .expect("server request timeout")
        .expect("server request closed");
    assert_eq!(req.method, "item/tool/requestUserInput");

    let invalid = runtime
        .respond_approval_ok(&req.approval_id, json!({"decision":"cancel"}))
        .await;
    assert!(invalid.is_err(), "missing answers object must fail");

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn dynamic_tool_call_payload_validation_rejects_missing_content_items() {
    let runtime = spawn_mock_runtime().await;
    let mut server_request_rx = runtime
        .take_server_request_rx()
        .await
        .expect("take server request rx");

    runtime
        .call_raw("probe_dynamic_tool_call", json!({}))
        .await
        .expect("probe_dynamic_tool_call");
    let req = timeout(Duration::from_secs(2), server_request_rx.recv())
        .await
        .expect("server request timeout")
        .expect("server request closed");
    assert_eq!(req.method, "item/tool/call");

    let invalid = runtime
        .respond_approval_ok(&req.approval_id, json!({"success":true}))
        .await;
    assert!(invalid.is_err(), "missing contentItems must fail");

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn auth_refresh_payload_validation_rejects_missing_access_token() {
    let runtime = spawn_mock_runtime().await;
    let mut server_request_rx = runtime
        .take_server_request_rx()
        .await
        .expect("take server request rx");

    runtime
        .call_raw("probe_auth_refresh", json!({}))
        .await
        .expect("probe_auth_refresh");
    let req = timeout(Duration::from_secs(2), server_request_rx.recv())
        .await
        .expect("server request timeout")
        .expect("server request closed");
    assert_eq!(req.method, "account/chatgptAuthTokens/refresh");

    let invalid = runtime
        .respond_approval_ok(
            &req.approval_id,
            json!({
                "chatgptAccountId": "acct_1",
                "chatgptPlanType": "plus"
            }),
        )
        .await;
    assert!(invalid.is_err(), "missing accessToken must fail");

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn server_request_with_string_id_roundtrip() {
    let runtime = spawn_mock_runtime().await;
    let mut live_rx = runtime.subscribe_live();
    let mut server_request_rx = runtime
        .take_server_request_rx()
        .await
        .expect("take server request rx");

    runtime
        .call_raw("probe_string_id", json!({}))
        .await
        .expect("probe_string_id");
    let req = timeout(Duration::from_secs(2), server_request_rx.recv())
        .await
        .expect("server request timeout")
        .expect("server request closed");
    assert_eq!(req.method, "item/fileChange/requestApproval");

    let snapshot = runtime.state_snapshot();
    assert!(
        snapshot.pending_server_requests.contains_key("s:req_str_1"),
        "state must index string request id"
    );

    runtime
        .respond_approval_ok(&req.approval_id, json!({"decision":"decline"}))
        .await
        .expect("respond approval");

    let mut saw_server_request_envelope = false;
    let mut saw_ack = false;
    for _ in 0..12 {
        let envelope = timeout(Duration::from_secs(2), live_rx.recv())
            .await
            .expect("live timeout")
            .expect("live closed");
        if envelope.kind == MsgKind::ServerRequest
            && envelope.method.as_deref() == Some("item/fileChange/requestApproval")
        {
            assert_eq!(
                envelope.rpc_id,
                Some(JsonRpcId::Text("req_str_1".to_owned()))
            );
            saw_server_request_envelope = true;
        }
        if envelope.kind == MsgKind::Notification
            && envelope.method.as_deref() == Some("approval/ack")
            && envelope.json["params"]["approvalRpcId"] == "req_str_1"
        {
            assert_eq!(envelope.json["params"]["result"]["decision"], "decline");
            saw_ack = true;
            if saw_server_request_envelope {
                break;
            }
        }
    }
    assert!(saw_server_request_envelope);
    assert!(saw_ack);

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn unknown_server_request_is_auto_declined() {
    let runtime = spawn_mock_runtime().await;
    let mut live_rx = runtime.subscribe_live();
    let mut server_request_rx = runtime
        .take_server_request_rx()
        .await
        .expect("take server request rx");

    runtime
        .call_raw("probe_unknown", json!({}))
        .await
        .expect("probe_unknown");

    let queued = timeout(Duration::from_millis(200), server_request_rx.recv()).await;
    assert!(queued.is_err(), "unknown request should not reach queue");

    let mut saw_ack = false;
    for _ in 0..8 {
        let envelope = timeout(Duration::from_secs(2), live_rx.recv())
            .await
            .expect("live timeout")
            .expect("live closed");
        if envelope.kind == MsgKind::Notification
            && envelope.method.as_deref() == Some("approval/ack")
            && envelope.json["params"]["approvalRpcId"] == 778
        {
            assert_eq!(envelope.json["params"]["result"]["decision"], "decline");
            saw_ack = true;
            break;
        }
    }

    assert!(saw_ack);
    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn timeout_policy_decline_replies_without_stall() {
    let runtime = spawn_mock_runtime_with_server_cfg(ServerRequestConfig {
        default_timeout_ms: 50,
        on_timeout: TimeoutAction::Decline,
        auto_decline_unknown: true,
    })
    .await;
    let mut live_rx = runtime.subscribe_live();
    let mut server_request_rx = runtime
        .take_server_request_rx()
        .await
        .expect("take server request rx");

    runtime
        .call_raw("probe_timeout", json!({}))
        .await
        .expect("probe_timeout");

    let req = timeout(Duration::from_secs(2), server_request_rx.recv())
        .await
        .expect("server request timeout")
        .expect("server request closed");
    assert_eq!(req.method, "item/fileChange/requestApproval");

    let mut saw_ack = false;
    for _ in 0..16 {
        let envelope = timeout(Duration::from_secs(2), live_rx.recv())
            .await
            .expect("live timeout")
            .expect("live closed");
        if envelope.kind == MsgKind::Notification
            && envelope.method.as_deref() == Some("approval/ack")
            && envelope.json["params"]["approvalRpcId"] == 779
        {
            assert_eq!(envelope.json["params"]["result"]["decision"], "decline");
            saw_ack = true;
            break;
        }
    }

    assert!(saw_ack);
    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn timeout_policy_decline_returns_empty_answers_for_user_input() {
    let runtime = spawn_mock_runtime_with_server_cfg(ServerRequestConfig {
        default_timeout_ms: 50,
        on_timeout: TimeoutAction::Decline,
        auto_decline_unknown: true,
    })
    .await;
    let mut live_rx = runtime.subscribe_live();
    let mut server_request_rx = runtime
        .take_server_request_rx()
        .await
        .expect("take server request rx");

    runtime
        .call_raw("probe_user_input", json!({}))
        .await
        .expect("probe_user_input");
    let req = timeout(Duration::from_secs(2), server_request_rx.recv())
        .await
        .expect("server request timeout")
        .expect("server request closed");
    assert_eq!(req.method, "item/tool/requestUserInput");

    let mut saw_ack = false;
    for _ in 0..16 {
        let envelope = timeout(Duration::from_secs(2), live_rx.recv())
            .await
            .expect("live timeout")
            .expect("live closed");
        if envelope.kind == MsgKind::Notification
            && envelope.method.as_deref() == Some("approval/ack")
            && envelope.json["params"]["approvalRpcId"] == 780
        {
            assert!(envelope.json["params"]["result"]["answers"].is_object());
            assert_eq!(
                envelope.json["params"]["result"]["answers"]
                    .as_object()
                    .expect("answers object")
                    .len(),
                0
            );
            saw_ack = true;
            break;
        }
    }

    assert!(saw_ack);
    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn timeout_policy_decline_returns_failure_payload_for_dynamic_tool_call() {
    let runtime = spawn_mock_runtime_with_server_cfg(ServerRequestConfig {
        default_timeout_ms: 50,
        on_timeout: TimeoutAction::Decline,
        auto_decline_unknown: true,
    })
    .await;
    let mut live_rx = runtime.subscribe_live();
    let mut server_request_rx = runtime
        .take_server_request_rx()
        .await
        .expect("take server request rx");

    runtime
        .call_raw("probe_dynamic_tool_call", json!({}))
        .await
        .expect("probe_dynamic_tool_call");
    let req = timeout(Duration::from_secs(2), server_request_rx.recv())
        .await
        .expect("server request timeout")
        .expect("server request closed");
    assert_eq!(req.method, "item/tool/call");

    let mut saw_ack = false;
    for _ in 0..16 {
        let envelope = timeout(Duration::from_secs(2), live_rx.recv())
            .await
            .expect("live timeout")
            .expect("live closed");
        if envelope.kind == MsgKind::Notification
            && envelope.method.as_deref() == Some("approval/ack")
            && envelope.json["params"]["approvalRpcId"] == 781
        {
            assert_eq!(envelope.json["params"]["result"]["success"], false);
            assert_eq!(envelope.json["params"]["result"]["contentItems"], json!([]));
            saw_ack = true;
            break;
        }
    }

    assert!(saw_ack);
    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn timeout_policy_decline_returns_error_for_auth_refresh() {
    let runtime = spawn_mock_runtime_with_server_cfg(ServerRequestConfig {
        default_timeout_ms: 50,
        on_timeout: TimeoutAction::Decline,
        auto_decline_unknown: true,
    })
    .await;
    let mut live_rx = runtime.subscribe_live();
    let mut server_request_rx = runtime
        .take_server_request_rx()
        .await
        .expect("take server request rx");

    runtime
        .call_raw("probe_auth_refresh", json!({}))
        .await
        .expect("probe_auth_refresh");
    let req = timeout(Duration::from_secs(2), server_request_rx.recv())
        .await
        .expect("server request timeout")
        .expect("server request closed");
    assert_eq!(req.method, "account/chatgptAuthTokens/refresh");

    let mut saw_ack = false;
    for _ in 0..16 {
        let envelope = timeout(Duration::from_secs(2), live_rx.recv())
            .await
            .expect("live timeout")
            .expect("live closed");
        if envelope.kind == MsgKind::Notification
            && envelope.method.as_deref() == Some("approval/ack")
            && envelope.json["params"]["approvalRpcId"] == 782
        {
            assert_eq!(envelope.json["params"]["error"]["code"], -32000);
            assert_eq!(
                envelope.json["params"]["error"]["data"]["method"],
                "account/chatgptAuthTokens/refresh"
            );
            saw_ack = true;
            break;
        }
    }

    assert!(saw_ack);
    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn closed_server_request_queue_resolves_immediately() {
    let runtime = spawn_mock_runtime_with_server_cfg(ServerRequestConfig {
        default_timeout_ms: 30_000,
        on_timeout: TimeoutAction::Decline,
        auto_decline_unknown: true,
    })
    .await;
    let mut live_rx = runtime.subscribe_live();

    let server_request_rx = runtime
        .take_server_request_rx()
        .await
        .expect("take server request rx");
    drop(server_request_rx);

    let started = std::time::Instant::now();
    runtime
        .call_raw("probe_timeout", json!({}))
        .await
        .expect("probe_timeout");

    let mut saw_ack = false;
    for _ in 0..16 {
        let envelope = timeout(Duration::from_secs(2), live_rx.recv())
            .await
            .expect("live timeout")
            .expect("live closed");
        if envelope.kind == MsgKind::Notification
            && envelope.method.as_deref() == Some("approval/ack")
            && envelope.json["params"]["approvalRpcId"] == 779
        {
            assert_eq!(envelope.json["params"]["result"]["decision"], "decline");
            saw_ack = true;
            break;
        }
    }

    assert!(saw_ack);
    assert!(started.elapsed() < Duration::from_secs(1));
    let snapshot = runtime.state_snapshot();
    assert!(snapshot.pending_server_requests.is_empty());
    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn call_raw_fails_when_not_initialized() {
    let runtime = spawn_mock_runtime().await;
    runtime.shutdown().await.expect("shutdown");

    let err = runtime
        .call_raw("echo/test", json!({}))
        .await
        .expect_err("must fail");
    assert!(matches!(err, RpcError::InvalidRequest(_)));
}

#[tokio::test(flavor = "current_thread")]
async fn state_snapshot_tracks_lifecycle_without_copy_on_read() {
    let runtime = spawn_mock_runtime().await;

    let before_a = runtime.state_snapshot();
    let before_b = runtime.state_snapshot();
    assert!(Arc::ptr_eq(&before_a, &before_b));
    assert_eq!(
        before_a.connection,
        ConnectionState::Running { generation: 0 }
    );

    runtime
        .call_raw("probe_state", json!({}))
        .await
        .expect("probe_state");

    let after = runtime.state_snapshot();
    let thread = after.threads.get("thr_state").expect("thread");
    assert_eq!(thread.active_turn, None);

    let turn = thread.turns.get("turn_state").expect("turn");
    assert_eq!(turn.status, crate::state::TurnStatus::Completed);
    let item = turn.items.get("item_state").expect("item");
    assert_eq!(item.text_accum, "hello");

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn state_snapshot_contains_pending_server_requests() {
    let runtime = spawn_mock_runtime_with_server_cfg(ServerRequestConfig {
        default_timeout_ms: 2_000,
        on_timeout: TimeoutAction::Decline,
        auto_decline_unknown: true,
    })
    .await;
    let mut server_request_rx = runtime
        .take_server_request_rx()
        .await
        .expect("take server request rx");

    runtime.call_raw("probe", json!({})).await.expect("probe");
    let req = timeout(Duration::from_secs(2), server_request_rx.recv())
        .await
        .expect("server request timeout")
        .expect("server request closed");

    let mid = runtime.state_snapshot();
    assert!(mid
        .pending_server_requests
        .values()
        .any(|v| v.approval_id == req.approval_id));

    runtime
        .respond_approval_ok(&req.approval_id, json!({"decision":"accept"}))
        .await
        .expect("respond approval");
    let after = runtime.state_snapshot();
    assert!(!after
        .pending_server_requests
        .values()
        .any(|v| v.approval_id == req.approval_id));

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn spawn_fails_when_metadata_missing_required_field() {
    let fixture = make_temp_schema_fixture(
        r#"{
  "schemaName":"app-server",
  "generatorCommand":"codex app-server generate-json-schema --out <DIR>",
  "sourceOfTruth":"active/json-schema"
}"#,
        &[],
        Some(""),
    );

    let cfg = RuntimeConfig::new(python_mock_process(), fixture.guard());
    let err = match Runtime::spawn_local(cfg).await {
        Ok(_) => panic!("spawn must fail"),
        Err(err) => err,
    };

    match err {
        RuntimeError::Internal(msg) => {
            assert!(msg.contains("metadata"), "unexpected error message: {msg}");
        }
        other => panic!("unexpected error type: {other:?}"),
    }
}

#[tokio::test(flavor = "current_thread")]
async fn spawn_fails_when_manifest_mismatches_schema_files() {
    let fixture = make_temp_schema_fixture(
        r#"{
  "schemaName":"app-server",
  "generatedAtUtc":"2026-01-01T00:00:00Z",
  "generatorCommand":"codex app-server generate-json-schema --out <DIR>",
  "sourceOfTruth":"active/json-schema"
}"#,
        &[("root.json", br#"{"type":"object"}"#)],
        Some("deadbeef  ./root.json"),
    );

    let cfg = RuntimeConfig::new(python_mock_process(), fixture.guard());
    let err = match Runtime::spawn_local(cfg).await {
        Ok(_) => panic!("spawn must fail"),
        Err(err) => err,
    };

    match err {
        RuntimeError::Internal(msg) => {
            assert!(msg.contains("manifest"), "unexpected error message: {msg}");
        }
        other => panic!("unexpected error type: {other:?}"),
    }
}
