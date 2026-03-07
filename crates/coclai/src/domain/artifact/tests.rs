use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::*;
use crate::plugin::PluginContractVersion;
use crate::runtime::core::RuntimeConfig;
use crate::runtime::errors::RuntimeError;
use crate::runtime::events::{Direction, Envelope, MsgKind};
use crate::runtime::transport::StdioProcessSpec;
use crate::runtime::turn_output::{parse_thread_id, parse_turn_id};
use serde_json::json;
use serde_json::Value;

#[derive(Debug)]
struct TempDir {
    root: PathBuf,
}

impl TempDir {
    fn new(prefix: &str) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("{prefix}_{now}"));
        fs::create_dir_all(&root).expect("create temp dir");
        Self { root }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn mock_runtime_process() -> StdioProcessSpec {
    let script = r###"
import json
import re
import sys

def extract_goal(text):
    m = re.search(r"GOAL:\n(.*?)\n\nCONSTRAINTS:", text, re.S)
    return m.group(1).strip() if m else ""

def extract_revision(text):
    m = re.search(r"REVISION:\s*(\S+)", text)
    return m.group(1).strip() if m else "sha256:missing"

for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    try:
        msg = json.loads(line)
    except Exception:
        continue

    rpc_id = msg.get("id")
    method = msg.get("method")
    params = msg.get("params") or {}

    if rpc_id is None:
        continue

    if method == "initialize":
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"ready": True}}) + "\n")
        sys.stdout.flush()
        continue

    if method == "thread/start":
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"thread": {"id": "thr_art"}}}) + "\n")
        sys.stdout.flush()
        continue

    if method == "thread/resume":
        thread_id = params.get("threadId") or "thr_art"
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"thread": {"id": thread_id}}}) + "\n")
        sys.stdout.flush()
        continue

    if method == "turn/start":
        input_items = params.get("input") or []
        input_text = ""
        if len(input_items) > 0:
            input_text = input_items[0].get("text") or ""

        goal = extract_goal(input_text)
        revision = extract_revision(input_text)

        if goal == "GENERATE_DOC":
            payload = {
                "format": "markdown",
                "title": "Generated Title",
                "text": "# Generated\ncontent\n"
            }
        elif goal == "EDIT_DOC":
            payload = {
                "format": "markdown",
                "expectedRevision": revision,
                "edits": [
                    {"startLine": 2, "endLine": 3, "replacement": "patched\n"}
                ],
                "notes": "ok"
            }
        elif goal == "EDIT_CONFLICT":
            payload = {
                "format": "markdown",
                "expectedRevision": "sha256:deadbeef",
                "edits": [
                    {"startLine": 1, "endLine": 2, "replacement": "boom\n"}
                ],
                "notes": "conflict"
            }
        elif goal == "POLICY_CHECK":
            payload = {
                "approvalPolicy": params.get("approvalPolicy"),
                "sandboxPolicy": params.get("sandboxPolicy")
            }
        else:
            payload = {"ok": True}

        payload_json = json.dumps(payload)
        turn_id = "turn_1"
        thread_id = params.get("threadId", "thr_art")
        sys.stdout.write(json.dumps({"method":"turn/started","params":{"threadId":thread_id,"turnId":turn_id}}) + "\n")
        sys.stdout.write(json.dumps({"method":"item/started","params":{"threadId":thread_id,"turnId":turn_id,"itemId":"item_1","itemType":"agentMessage"}}) + "\n")
        sys.stdout.write(json.dumps({"method":"item/agentMessage/delta","params":{"threadId":thread_id,"turnId":turn_id,"itemId":"item_1","delta":payload_json}}) + "\n")
        sys.stdout.write(json.dumps({"method":"item/completed","params":{"threadId":thread_id,"turnId":turn_id,"itemId":"item_1","item":{"type":"agent_message","text":payload_json}}}) + "\n")
        sys.stdout.write(json.dumps({"method":"turn/completed","params":{"threadId":thread_id,"turnId":turn_id}}) + "\n")
        out = {"id": rpc_id, "result": {"turn": {"id": turn_id, "status": "inProgress", "items": []}}}
        sys.stdout.write(json.dumps(out) + "\n")
        sys.stdout.flush()
        continue

    sys.stdout.write(json.dumps({"id": rpc_id, "result": {"ok": True}}) + "\n")
    sys.stdout.flush()
"###;

    crate::test_fixtures::python_inline_process(script)
}

fn interrupt_probe_runtime_process(interrupt_mark: &str) -> StdioProcessSpec {
    let script = r###"
import json
import os
import sys

mark = os.environ.get("INTERRUPT_MARK")

for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    try:
        msg = json.loads(line)
    except Exception:
        continue

    rpc_id = msg.get("id")
    method = msg.get("method")
    params = msg.get("params") or {}

    if method == "turn/interrupt":
        if mark:
            with open(mark, "w", encoding="utf-8") as f:
                f.write("seen")
        if rpc_id is not None:
            sys.stdout.write(json.dumps({"id": rpc_id, "result": {"ok": True}}) + "\n")
            sys.stdout.flush()
        continue

    if rpc_id is None:
        continue

    if method == "initialize":
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"ready": True}}) + "\n")
        sys.stdout.flush()
        continue

    if method == "thread/start":
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"thread": {"id": "thr_art"}}}) + "\n")
        sys.stdout.flush()
        continue

    if method == "thread/resume":
        thread_id = params.get("threadId") or "thr_art"
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"thread": {"id": thread_id}}}) + "\n")
        sys.stdout.flush()
        continue

    if method == "turn/start":
        thread_id = params.get("threadId", "thr_art")
        turn_id = "turn_hot"
        input_items = params.get("input") or []
        input_text = ""
        if len(input_items) > 0 and isinstance(input_items[0], dict):
            input_text = input_items[0].get("text") or ""
        if "DIRECT_OUTPUT_PARSE_FAIL" in input_text:
            sys.stdout.write(json.dumps({
                "id": rpc_id,
                "result": {
                    "turn": {"id": turn_id},
                    "output": "{not-json"
                }
            }) + "\n")
            sys.stdout.flush()
            continue
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"turn": {"id": turn_id}}}) + "\n")
        for _ in range(21050):
            sys.stdout.write(json.dumps({
                "method":"item/agentMessage/delta",
                "params":{"threadId":thread_id,"turnId":turn_id,"itemId":"item_hot","delta":"x"}
            }) + "\n")
        sys.stdout.flush()
        continue

    sys.stdout.write(json.dumps({"id": rpc_id, "result": {"ok": True}}) + "\n")
    sys.stdout.flush()
"###;

    let mut spec = crate::test_fixtures::python_inline_process(script);
    spec.env
        .insert("INTERRUPT_MARK".to_owned(), interrupt_mark.to_owned());
    spec
}

fn resume_missing_id_runtime_process() -> StdioProcessSpec {
    let script = r###"
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

    rpc_id = msg.get("id")
    method = msg.get("method")
    params = msg.get("params") or {}

    if rpc_id is None:
        continue

    if method == "initialize":
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"ready": True}}) + "\n")
        sys.stdout.flush()
        continue

    if method == "thread/start":
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"thread": {"id": "thr_art_new"}}}) + "\n")
        sys.stdout.flush()
        continue

    if method == "thread/resume":
        # Deliberately omit any thread id field to validate strict client contract handling.
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"ok": True}}) + "\n")
        sys.stdout.flush()
        continue

    sys.stdout.write(json.dumps({"id": rpc_id, "result": {"ok": True, "echoMethod": method, "params": params}}) + "\n")
    sys.stdout.flush()
"###;

    crate::test_fixtures::python_inline_process(script)
}

fn resume_mismatched_id_runtime_process() -> StdioProcessSpec {
    let script = r###"
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

    rpc_id = msg.get("id")
    method = msg.get("method")
    params = msg.get("params") or {}

    if rpc_id is None:
        continue

    if method == "initialize":
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"ready": True}}) + "\n")
        sys.stdout.flush()
        continue

    if method == "thread/start":
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"thread": {"id": "thr_art_new"}}}) + "\n")
        sys.stdout.flush()
        continue

    if method == "thread/resume":
        # Deliberately return a different id to validate strict resume contract handling.
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"thread": {"id": "thr_unexpected"}}}) + "\n")
        sys.stdout.flush()
        continue

    sys.stdout.write(json.dumps({"id": rpc_id, "result": {"ok": True, "echoMethod": method, "params": params}}) + "\n")
    sys.stdout.flush()
"###;

    crate::test_fixtures::python_inline_process(script)
}

async fn spawn_mock_runtime() -> Runtime {
    let cfg = RuntimeConfig::new(mock_runtime_process());
    Runtime::spawn_local(cfg).await.expect("runtime spawn")
}

async fn spawn_interrupt_probe_runtime(interrupt_mark: &str) -> Runtime {
    let cfg = RuntimeConfig::new(interrupt_probe_runtime_process(interrupt_mark));
    Runtime::spawn_local(cfg).await.expect("runtime spawn")
}

async fn spawn_resume_missing_id_runtime() -> Runtime {
    let cfg = RuntimeConfig::new(resume_missing_id_runtime_process());
    Runtime::spawn_local(cfg).await.expect("runtime spawn")
}

async fn spawn_resume_mismatched_id_runtime() -> Runtime {
    let cfg = RuntimeConfig::new(resume_mismatched_id_runtime_process());
    Runtime::spawn_local(cfg).await.expect("runtime spawn")
}

fn make_task_spec(artifact_id: &str, kind: ArtifactTaskKind, goal: &str) -> ArtifactTaskSpec {
    ArtifactTaskSpec {
        artifact_id: artifact_id.to_owned(),
        kind,
        user_goal: goal.to_owned(),
        current_text: None,
        constraints: vec!["Keep output deterministic".to_owned()],
        examples: vec![],
        model: None,
        effort: None,
        summary: None,
        output_schema: json!({"type":"object"}),
    }
}

#[derive(Clone)]
struct FakeArtifactAdapter {
    state: Arc<Mutex<FakeArtifactAdapterState>>,
}

#[derive(Default, Debug)]
struct FakeArtifactAdapterState {
    start_thread_id: String,
    turn_output: Value,
    turn_id: Option<String>,
    start_calls: usize,
    resume_calls: Vec<String>,
    run_turn_calls: Vec<(String, String, ArtifactTaskSpec)>,
}

impl ArtifactPluginAdapter for FakeArtifactAdapter {
    fn start_thread<'a>(&'a self) -> ArtifactAdapterFuture<'a, Result<String, DomainError>> {
        Box::pin(async move {
            let mut state = self.state.lock().expect("fake adapter lock");
            state.start_calls += 1;
            Ok(state.start_thread_id.clone())
        })
    }

    fn resume_thread<'a>(
        &'a self,
        thread_id: &'a str,
    ) -> ArtifactAdapterFuture<'a, Result<String, DomainError>> {
        Box::pin(async move {
            let mut state = self.state.lock().expect("fake adapter lock");
            state.resume_calls.push(thread_id.to_owned());
            Ok(thread_id.to_owned())
        })
    }

    fn run_turn<'a>(
        &'a self,
        thread_id: &'a str,
        prompt: &'a str,
        spec: &'a ArtifactTaskSpec,
    ) -> ArtifactAdapterFuture<'a, Result<ArtifactTurnOutput, DomainError>> {
        Box::pin(async move {
            let mut state = self.state.lock().expect("fake adapter lock");
            state
                .run_turn_calls
                .push((thread_id.to_owned(), prompt.to_owned(), spec.clone()));
            Ok(ArtifactTurnOutput {
                turn_id: state.turn_id.clone(),
                output: state.turn_output.clone(),
            })
        })
    }
}

#[derive(Clone)]
struct IncompatibleArtifactAdapter;

impl ArtifactPluginAdapter for IncompatibleArtifactAdapter {
    fn plugin_contract_version(&self) -> PluginContractVersion {
        PluginContractVersion::new(2, 0)
    }

    fn start_thread<'a>(&'a self) -> ArtifactAdapterFuture<'a, Result<String, DomainError>> {
        Box::pin(async move { panic!("start_thread must not be called on incompatible adapter") })
    }

    fn resume_thread<'a>(
        &'a self,
        _thread_id: &'a str,
    ) -> ArtifactAdapterFuture<'a, Result<String, DomainError>> {
        Box::pin(async move { panic!("resume_thread must not be called on incompatible adapter") })
    }

    fn run_turn<'a>(
        &'a self,
        _thread_id: &'a str,
        _prompt: &'a str,
        _spec: &'a ArtifactTaskSpec,
    ) -> ArtifactAdapterFuture<'a, Result<ArtifactTurnOutput, DomainError>> {
        Box::pin(async move { panic!("run_turn must not be called on incompatible adapter") })
    }
}

#[derive(Clone)]
struct CompatibleMinorArtifactAdapter;

impl ArtifactPluginAdapter for CompatibleMinorArtifactAdapter {
    fn plugin_contract_version(&self) -> PluginContractVersion {
        PluginContractVersion::new(1, 42)
    }

    fn start_thread<'a>(&'a self) -> ArtifactAdapterFuture<'a, Result<String, DomainError>> {
        Box::pin(async move { Ok("thr_contract_minor".to_owned()) })
    }

    fn resume_thread<'a>(
        &'a self,
        _thread_id: &'a str,
    ) -> ArtifactAdapterFuture<'a, Result<String, DomainError>> {
        Box::pin(async move { panic!("resume_thread is not expected for compatibility-open test") })
    }

    fn run_turn<'a>(
        &'a self,
        _thread_id: &'a str,
        _prompt: &'a str,
        _spec: &'a ArtifactTaskSpec,
    ) -> ArtifactAdapterFuture<'a, Result<ArtifactTurnOutput, DomainError>> {
        Box::pin(async move { panic!("run_turn is not expected for compatibility-open test") })
    }
}

fn seed_artifact(store: &dyn ArtifactStore, artifact_id: &str, text: &str) {
    let revision = compute_revision(text);
    store
        .save_text(
            artifact_id,
            text,
            SaveMeta {
                task_kind: ArtifactTaskKind::DocGenerate,
                thread_id: "seed".to_owned(),
                turn_id: None,
                previous_revision: None,
                next_revision: revision.clone(),
            },
        )
        .expect("seed text");
    store
        .set_meta(
            artifact_id,
            ArtifactMeta {
                title: "Seed".to_owned(),
                format: "markdown".to_owned(),
                revision,
                runtime_thread_id: None,
            },
        )
        .expect("seed meta");
}

fn envelope_for_turn(method: &str, thread_id: &str, turn_id: &str, params: Value) -> Envelope {
    Envelope {
        seq: 1,
        ts_millis: 0,
        direction: Direction::Inbound,
        kind: MsgKind::Notification,
        rpc_id: None,
        method: Some(Arc::from(method)),
        thread_id: Some(Arc::from(thread_id)),
        turn_id: Some(Arc::from(turn_id)),
        item_id: None,
        json: Arc::new(json!({
            "method": method,
            "params": params
        })),
    }
}

// Unit: pure document transforms/store invariants.
#[path = "tests/unit_core.rs"]
mod unit_core;
// Contract: turn output collection/terminal semantics.
#[path = "tests/collect_output.rs"]
mod collect_output;
// Integration: runtime + store task execution flows.
#[path = "tests/runtime_tasks.rs"]
mod runtime_tasks;
