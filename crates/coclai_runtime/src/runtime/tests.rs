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

#[path = "tests/core_lifecycle.rs"]
mod core_lifecycle;
#[path = "tests/server_requests.rs"]
mod server_requests;
#[path = "tests/state_and_schema.rs"]
mod state_and_schema;
