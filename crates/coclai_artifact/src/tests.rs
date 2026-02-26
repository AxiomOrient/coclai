use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::*;
use coclai_runtime::events::{Direction, Envelope, MsgKind};
use coclai_runtime::runtime::{RuntimeConfig, SchemaGuardConfig};
use coclai_runtime::transport::StdioProcessSpec;
use coclai_runtime::turn_output::{parse_thread_id, parse_turn_id};
use coclai_runtime::PluginContractVersion;
use pretty_assertions::assert_eq;
use serde_json::json;

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

fn workspace_schema_guard() -> SchemaGuardConfig {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let active = manifest_dir.join("../../SCHEMAS/app-server/active");
    SchemaGuardConfig {
        active_schema_dir: active,
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

    let mut spec = StdioProcessSpec::new("python3");
    spec.args = vec!["-u".to_owned(), "-c".to_owned(), script.to_owned()];
    spec
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

    let mut spec = StdioProcessSpec::new("python3");
    spec.args = vec!["-u".to_owned(), "-c".to_owned(), script.to_owned()];
    spec.env
        .insert("INTERRUPT_MARK".to_owned(), interrupt_mark.to_owned());
    spec
}

async fn spawn_mock_runtime() -> Runtime {
    let cfg = RuntimeConfig::new(mock_runtime_process(), workspace_schema_guard());
    Runtime::spawn_local(cfg).await.expect("runtime spawn")
}

async fn spawn_interrupt_probe_runtime(interrupt_mark: &str) -> Runtime {
    let cfg = RuntimeConfig::new(
        interrupt_probe_runtime_process(interrupt_mark),
        workspace_schema_guard(),
    );
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

#[tokio::test(flavor = "current_thread")]
async fn run_task_doc_generate_end_to_end() {
    let temp = TempDir::new("coclai_artifact_generate");
    let store: Arc<dyn ArtifactStore> = Arc::new(FsArtifactStore::new(&temp.root));
    seed_artifact(store.as_ref(), "doc:generate", "");

    let runtime = spawn_mock_runtime().await;
    let manager = ArtifactSessionManager::new(runtime.clone(), Arc::clone(&store));

    let spec = make_task_spec(
        "doc:generate",
        ArtifactTaskKind::DocGenerate,
        "GENERATE_DOC",
    );
    let result = manager.run_task(spec).await.expect("run task");

    match result {
        ArtifactTaskResult::DocGenerate {
            revision,
            text,
            title,
            ..
        } => {
            assert_eq!(title, "Generated Title");
            assert_eq!(text, "# Generated\ncontent\n");
            assert_eq!(revision, compute_revision(&text));
        }
        other => panic!("unexpected result: {other:?}"),
    }

    let persisted = store.load_text("doc:generate").expect("load persisted");
    assert_eq!(persisted, "# Generated\ncontent\n");

    let meta = store.get_meta("doc:generate").expect("load meta");
    assert_eq!(meta.title, "Generated Title");
    assert_eq!(meta.format, "markdown");
    assert_eq!(meta.revision, compute_revision(&persisted));
    assert_eq!(meta.runtime_thread_id.as_deref(), Some("thr_art"));

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn run_task_uses_artifact_adapter_boundary_without_runtime_dependency() {
    let temp = TempDir::new("coclai_artifact_fake_adapter");
    let store: Arc<dyn ArtifactStore> = Arc::new(FsArtifactStore::new(&temp.root));
    seed_artifact(store.as_ref(), "doc:adapter", "");

    let state = Arc::new(Mutex::new(FakeArtifactAdapterState {
        start_thread_id: "thr_fake_adapter".to_owned(),
        turn_output: json!({
            "format": "markdown",
            "title": "Adapter Title",
            "text": "# Adapter\nok\n"
        }),
        turn_id: Some("turn_fake_adapter".to_owned()),
        ..FakeArtifactAdapterState::default()
    }));
    let adapter: Arc<dyn ArtifactPluginAdapter> = Arc::new(FakeArtifactAdapter {
        state: Arc::clone(&state),
    });
    let manager = ArtifactSessionManager::new_with_adapter(adapter, Arc::clone(&store));

    let spec = make_task_spec("doc:adapter", ArtifactTaskKind::DocGenerate, "GENERATE_DOC");
    let result = manager
        .run_task(spec)
        .await
        .expect("run task with fake adapter");

    match result {
        ArtifactTaskResult::DocGenerate {
            thread_id,
            turn_id,
            title,
            text,
            ..
        } => {
            assert_eq!(thread_id, "thr_fake_adapter");
            assert_eq!(turn_id.as_deref(), Some("turn_fake_adapter"));
            assert_eq!(title, "Adapter Title");
            assert_eq!(text, "# Adapter\nok\n");
        }
        other => panic!("unexpected result: {other:?}"),
    }

    let state = state.lock().expect("fake adapter state");
    assert_eq!(state.start_calls, 1);
    assert!(state.resume_calls.is_empty());
    assert_eq!(state.run_turn_calls.len(), 1);
    let (thread_id, prompt, seen_spec) = &state.run_turn_calls[0];
    assert_eq!(thread_id, "thr_fake_adapter");
    assert!(prompt.contains("GOAL:\nGENERATE_DOC"));
    assert_eq!(seen_spec.artifact_id, "doc:adapter");
}

#[tokio::test(flavor = "current_thread")]
async fn run_task_doc_edit_end_to_end() {
    let temp = TempDir::new("coclai_artifact_edit");
    let store: Arc<dyn ArtifactStore> = Arc::new(FsArtifactStore::new(&temp.root));
    seed_artifact(store.as_ref(), "doc:edit", "a\nb\nc\n");

    let runtime = spawn_mock_runtime().await;
    let manager = ArtifactSessionManager::new(runtime.clone(), Arc::clone(&store));

    let spec = make_task_spec("doc:edit", ArtifactTaskKind::DocEdit, "EDIT_DOC");
    let result = manager.run_task(spec).await.expect("run task");

    match result {
        ArtifactTaskResult::DocEdit {
            text,
            notes,
            revision,
            ..
        } => {
            assert_eq!(text, "a\npatched\nc\n");
            assert_eq!(notes.as_deref(), Some("ok"));
            assert_eq!(revision, compute_revision("a\npatched\nc\n"));
        }
        other => panic!("unexpected result: {other:?}"),
    }

    let persisted = store.load_text("doc:edit").expect("load persisted");
    assert_eq!(persisted, "a\npatched\nc\n");

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn open_rejects_incompatible_adapter_contract() {
    let temp = TempDir::new("coclai_artifact_contract_mismatch");
    let store: Arc<dyn ArtifactStore> = Arc::new(FsArtifactStore::new(&temp.root));
    seed_artifact(store.as_ref(), "doc:contract", "seed\n");

    let adapter: Arc<dyn ArtifactPluginAdapter> = Arc::new(IncompatibleArtifactAdapter);
    let manager = ArtifactSessionManager::new_with_adapter(adapter, Arc::clone(&store));

    let err = manager
        .open("doc:contract")
        .await
        .expect_err("must reject mismatch");
    assert_eq!(
        err,
        DomainError::IncompatibleContract {
            expected_major: 1,
            expected_minor: 0,
            actual_major: 2,
            actual_minor: 0,
        }
    );
}

#[tokio::test(flavor = "current_thread")]
async fn open_accepts_compatible_minor_contract_version() {
    let temp = TempDir::new("coclai_artifact_contract_minor");
    let store: Arc<dyn ArtifactStore> = Arc::new(FsArtifactStore::new(&temp.root));
    seed_artifact(store.as_ref(), "doc:contract-minor", "seed\n");

    let adapter: Arc<dyn ArtifactPluginAdapter> = Arc::new(CompatibleMinorArtifactAdapter);
    let manager = ArtifactSessionManager::new_with_adapter(adapter, Arc::clone(&store));
    let session = manager
        .open("doc:contract-minor")
        .await
        .expect("minor version must remain compatible");

    assert_eq!(session.thread_id, "thr_contract_minor");
    assert_eq!(session.artifact_id, "doc:contract-minor");
}

#[tokio::test(flavor = "current_thread")]
async fn conflict_is_returned_without_auto_retry() {
    let temp = TempDir::new("coclai_artifact_conflict");
    let store: Arc<dyn ArtifactStore> = Arc::new(FsArtifactStore::new(&temp.root));
    seed_artifact(store.as_ref(), "doc:conflict", "a\nb\nc\n");

    let runtime = spawn_mock_runtime().await;
    let manager = ArtifactSessionManager::new(runtime.clone(), Arc::clone(&store));

    let spec = make_task_spec("doc:conflict", ArtifactTaskKind::DocEdit, "EDIT_CONFLICT");
    let err = manager.run_task(spec).await.expect_err("must conflict");

    match err {
        DomainError::Conflict { expected, actual } => {
            assert_eq!(expected, "sha256:deadbeef");
            assert_eq!(actual, compute_revision("a\nb\nc\n"));
        }
        other => panic!("unexpected error: {other:?}"),
    }

    let persisted = store.load_text("doc:conflict").expect("load persisted");
    assert_eq!(persisted, "a\nb\nc\n");

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn turn_start_params_use_fixed_safe_policy() {
    let temp = TempDir::new("coclai_artifact_policy");
    let store: Arc<dyn ArtifactStore> = Arc::new(FsArtifactStore::new(&temp.root));
    seed_artifact(store.as_ref(), "doc:policy", "seed\n");

    let runtime = spawn_mock_runtime().await;
    let manager = ArtifactSessionManager::new(runtime.clone(), Arc::clone(&store));

    let spec = make_task_spec("doc:policy", ArtifactTaskKind::Passthrough, "POLICY_CHECK");
    let result = manager.run_task(spec).await.expect("run task");

    match result {
        ArtifactTaskResult::Passthrough { output, .. } => {
            assert_eq!(output["approvalPolicy"], "never");
            assert_eq!(output["sandboxPolicy"]["type"], "readOnly");
        }
        other => panic!("unexpected result: {other:?}"),
    }

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn run_task_sends_interrupt_when_output_collection_fails() {
    let temp = TempDir::new("coclai_artifact_interrupt_probe");
    let store: Arc<dyn ArtifactStore> = Arc::new(FsArtifactStore::new(&temp.root));
    seed_artifact(store.as_ref(), "doc:interrupt", "seed\n");

    let interrupt_mark = temp.root.join("interrupt_seen.txt");
    let interrupt_mark_str = interrupt_mark.to_string_lossy().to_string();
    let runtime = spawn_interrupt_probe_runtime(&interrupt_mark_str).await;
    let manager = ArtifactSessionManager::new(runtime.clone(), Arc::clone(&store));

    let spec = make_task_spec(
        "doc:interrupt",
        ArtifactTaskKind::Passthrough,
        "INTERRUPT_CHECK",
    );
    let err = manager.run_task(spec).await.expect_err("must fail");
    assert!(matches!(err, DomainError::Parse(_)));
    assert!(
        interrupt_mark.exists(),
        "run_task failure must emit turn/interrupt best effort"
    );

    runtime.shutdown().await.expect("shutdown");
}

#[test]
fn build_prompt_has_required_blocks() {
    let spec = ArtifactTaskSpec {
        artifact_id: "doc:prompt".to_owned(),
        kind: ArtifactTaskKind::Passthrough,
        user_goal: "goal".to_owned(),
        current_text: None,
        constraints: vec!["c1".to_owned()],
        examples: vec!["ex1".to_owned()],
        model: None,
        effort: None,
        summary: None,
        output_schema: json!({"type":"object"}),
    };
    let prompt = build_turn_prompt(&spec, "markdown", "sha256:rev", "hello\n");
    assert!(prompt.contains("ROLE:\n"));
    assert!(prompt.contains("GOAL:\n"));
    assert!(prompt.contains("CONSTRAINTS:\n"));
    assert!(prompt.contains("CONTEXT:\n"));
    assert!(prompt.contains("REVISION: sha256:rev"));
    assert!(prompt.contains("CURRENT_TEXT_BEGIN\nhello\nCURRENT_TEXT_END"));
}

#[test]
fn parse_ids_support_nested_structures() {
    assert_eq!(
        parse_thread_id(&json!({"thread":{"id":"thr_nested"}})).as_deref(),
        Some("thr_nested")
    );
    assert_eq!(
        parse_turn_id(&json!({"turn":{"id":"turn_nested"}})).as_deref(),
        Some("turn_nested")
    );
}

#[test]
fn validate_and_apply_replace() {
    let before = "a\nb\nc\n";
    let patch = DocPatch {
        format: "markdown".to_owned(),
        expected_revision: compute_revision(before),
        edits: vec![DocEdit {
            start_line: 2,
            end_line: 3,
            replacement: "B\n".to_owned(),
        }],
        notes: None,
    };

    let validated = validate_doc_patch(before, &patch).expect("valid patch");
    let after = apply_doc_patch(before, &validated);
    assert_eq!(after, "a\nB\nc\n");
}

#[test]
fn validate_insert_head_and_append() {
    let before = "line1\nline2\n";
    let patch = DocPatch {
        format: "markdown".to_owned(),
        expected_revision: compute_revision(before),
        edits: vec![
            DocEdit {
                start_line: 1,
                end_line: 1,
                replacement: "head\n".to_owned(),
            },
            DocEdit {
                start_line: 3,
                end_line: 3,
                replacement: "tail\n".to_owned(),
            },
        ],
        notes: None,
    };

    let validated = validate_doc_patch(before, &patch).expect("valid patch");
    let after = apply_doc_patch(before, &validated);
    assert_eq!(after, "head\nline1\nline2\ntail\n");
}

#[test]
fn detect_revision_conflict() {
    let before = "a\n";
    let patch = DocPatch {
        format: "markdown".to_owned(),
        expected_revision: "sha256:deadbeef".to_owned(),
        edits: vec![],
        notes: None,
    };

    let err = validate_doc_patch(before, &patch).expect_err("must fail");
    assert!(matches!(err, PatchConflict::RevisionMismatch { .. }));
}

#[test]
fn detect_overlap() {
    let before = "a\nb\nc\n";
    let patch = DocPatch {
        format: "markdown".to_owned(),
        expected_revision: compute_revision(before),
        edits: vec![
            DocEdit {
                start_line: 1,
                end_line: 3,
                replacement: "x\n".to_owned(),
            },
            DocEdit {
                start_line: 2,
                end_line: 3,
                replacement: "y\n".to_owned(),
            },
        ],
        notes: None,
    };

    let err = validate_doc_patch(before, &patch).expect_err("must fail");
    assert!(matches!(err, PatchConflict::Overlap { .. }));
}

#[test]
fn detect_invalid_range() {
    let before = "a\n";
    let patch = DocPatch {
        format: "markdown".to_owned(),
        expected_revision: compute_revision(before),
        edits: vec![DocEdit {
            start_line: 2,
            end_line: 4,
            replacement: "x\n".to_owned(),
        }],
        notes: None,
    };

    let err = validate_doc_patch(before, &patch).expect_err("must fail");
    assert!(matches!(err, PatchConflict::InvalidRange { .. }));
}

#[test]
fn artifact_key_is_stable() {
    let a = artifact_key("doc:123");
    let b = artifact_key("doc:123");
    let c = artifact_key("doc/123");
    assert_eq!(a, b);
    assert_ne!(a, c);
}

#[test]
fn fs_store_rejects_stale_revision_on_save() {
    let temp = TempDir::new("coclai_artifact_store_conflict");
    let store = FsArtifactStore::new(&temp.root);

    let base_text = "v1\n";
    let base_revision = compute_revision(base_text);
    store
        .save_text(
            "doc:store-conflict",
            base_text,
            SaveMeta {
                task_kind: ArtifactTaskKind::DocGenerate,
                thread_id: "seed".to_owned(),
                turn_id: None,
                previous_revision: None,
                next_revision: base_revision.clone(),
            },
        )
        .expect("seed save");

    let next_text = "v2\n";
    let next_revision = compute_revision(next_text);
    store
        .save_text(
            "doc:store-conflict",
            next_text,
            SaveMeta {
                task_kind: ArtifactTaskKind::DocEdit,
                thread_id: "seed".to_owned(),
                turn_id: Some("turn_1".to_owned()),
                previous_revision: Some(base_revision.clone()),
                next_revision,
            },
        )
        .expect("first update");

    let stale = store
        .save_text(
            "doc:store-conflict",
            "v3\n",
            SaveMeta {
                task_kind: ArtifactTaskKind::DocEdit,
                thread_id: "seed".to_owned(),
                turn_id: Some("turn_2".to_owned()),
                previous_revision: Some(base_revision),
                next_revision: compute_revision("v3\n"),
            },
        )
        .expect_err("stale save must fail");
    assert!(matches!(stale, StoreErr::Conflict { .. }));
}

#[test]
fn fs_store_recovers_stale_lock_and_saves() {
    let temp = TempDir::new("coclai_artifact_store_stale_lock");
    let store = FsArtifactStore::new(&temp.root);
    let artifact_id = "doc:stale-lock";

    let artifact_dir = temp.root.join(artifact_key(artifact_id));
    fs::create_dir_all(&artifact_dir).expect("create artifact dir");
    fs::write(artifact_dir.join(".artifact.lock"), "0:1\n").expect("write stale lock");

    let next_text = "v1\n";
    let next_revision = compute_revision(next_text);
    store
        .save_text(
            artifact_id,
            next_text,
            SaveMeta {
                task_kind: ArtifactTaskKind::DocGenerate,
                thread_id: "seed".to_owned(),
                turn_id: None,
                previous_revision: None,
                next_revision: next_revision.clone(),
            },
        )
        .expect("save must recover stale lock");

    let persisted = store.load_text(artifact_id).expect("load persisted");
    assert_eq!(persisted, next_text);
    assert!(!artifact_dir.join(".artifact.lock").exists());
}

#[test]
fn fs_store_rejects_meta_revision_mismatch() {
    let temp = TempDir::new("coclai_artifact_store_meta_conflict");
    let store = FsArtifactStore::new(&temp.root);

    let text = "body\n";
    let revision = compute_revision(text);
    store
        .save_text(
            "doc:meta-conflict",
            text,
            SaveMeta {
                task_kind: ArtifactTaskKind::DocGenerate,
                thread_id: "seed".to_owned(),
                turn_id: None,
                previous_revision: None,
                next_revision: revision.clone(),
            },
        )
        .expect("seed save");

    let err = store
        .set_meta(
            "doc:meta-conflict",
            ArtifactMeta {
                title: "x".to_owned(),
                format: "markdown".to_owned(),
                revision: "sha256:deadbeef".to_owned(),
                runtime_thread_id: None,
            },
        )
        .expect_err("meta revision mismatch must fail");
    assert!(matches!(err, StoreErr::Conflict { .. }));
}

#[tokio::test(flavor = "current_thread")]
async fn open_repairs_meta_revision_mismatch() {
    let temp = TempDir::new("coclai_artifact_open_repair");
    let store: Arc<dyn ArtifactStore> = Arc::new(FsArtifactStore::new(&temp.root));

    let artifact_id = "doc:meta-mismatch";
    let key = artifact_key(artifact_id);
    let dir = temp.root.join(key);
    fs::create_dir_all(&dir).expect("create dir");
    fs::write(dir.join("text.txt"), "seed\n").expect("write text");
    fs::write(
        dir.join("meta.json"),
        serde_json::to_vec(&ArtifactMeta {
            title: "Seed".to_owned(),
            format: "markdown".to_owned(),
            revision: "sha256:bad".to_owned(),
            runtime_thread_id: None,
        })
        .expect("serialize meta"),
    )
    .expect("write meta");

    let runtime = spawn_mock_runtime().await;
    let manager = ArtifactSessionManager::new(runtime.clone(), Arc::clone(&store));
    let opened = manager.open(artifact_id).await.expect("open must repair");
    assert_eq!(opened.revision, compute_revision("seed\n"));

    let meta = store.get_meta(artifact_id).expect("meta after open");
    assert_eq!(meta.revision, compute_revision("seed\n"));
    assert_eq!(meta.runtime_thread_id.as_deref(), Some("thr_art"));

    runtime.shutdown().await.expect("shutdown");
}

#[test]
fn build_turn_start_params_uses_fixed_safe_policy() {
    let spec = ArtifactTaskSpec {
        artifact_id: "doc:unsafe".to_owned(),
        kind: ArtifactTaskKind::Passthrough,
        user_goal: "goal".to_owned(),
        current_text: None,
        constraints: vec![],
        examples: vec![],
        model: Some("m".to_owned()),
        effort: Some(ReasoningEffort::High),
        summary: Some("verbose".to_owned()),
        output_schema: json!({"type":"object"}),
    };
    let params = build_turn_start_params("thr_1", "prompt", &spec);
    assert_eq!(params["approvalPolicy"], "never");
    assert_eq!(params["sandboxPolicy"]["type"], "readOnly");
    assert_eq!(params["model"], "m");
    assert_eq!(params["effort"], "high");
    assert_eq!(params["summary"], "verbose");
}

#[test]
fn build_turn_start_params_defaults_effort_to_medium() {
    let spec = ArtifactTaskSpec {
        artifact_id: "doc:default-effort".to_owned(),
        kind: ArtifactTaskKind::Passthrough,
        user_goal: "goal".to_owned(),
        current_text: None,
        constraints: vec![],
        examples: vec![],
        model: None,
        effort: None,
        summary: None,
        output_schema: json!({"type":"object"}),
    };
    let params = build_turn_start_params("thr_1", "prompt", &spec);
    assert_eq!(params["effort"], "medium");
}

fn envelope_for_turn(method: &str, thread_id: &str, turn_id: &str, params: Value) -> Envelope {
    Envelope {
        seq: 1,
        ts_millis: 0,
        direction: Direction::Inbound,
        kind: MsgKind::Notification,
        rpc_id: None,
        method: Some(method.to_owned()),
        thread_id: Some(thread_id.to_owned()),
        turn_id: Some(turn_id.to_owned()),
        item_id: None,
        json: json!({
            "method": method,
            "params": params
        }),
    }
}

#[tokio::test(flavor = "current_thread")]
async fn collect_turn_output_times_out_without_matching_events() {
    let (_tx, mut rx) = tokio::sync::broadcast::channel::<Envelope>(8);

    let err = collect_turn_output_from_live_with_limits(
        &mut rx,
        "thr_timeout",
        "turn_timeout",
        8,
        Duration::from_millis(25),
    )
    .await
    .expect_err("must timeout");

    assert!(matches!(err, DomainError::Runtime(RuntimeError::Timeout)));
}

#[tokio::test(flavor = "current_thread")]
async fn collect_turn_output_budget_counts_only_matching_turn_events() {
    let (tx, mut rx) = tokio::sync::broadcast::channel::<Envelope>(32);

    for _ in 0..16 {
        tx.send(envelope_for_turn(
            "turn/completed",
            "thr_other",
            "turn_other",
            json!({"output":{"ignored": true}}),
        ))
        .expect("send unrelated");
    }

    tx.send(envelope_for_turn(
        "turn/completed",
        "thr_target",
        "turn_target",
        json!({"output":{"status":"ok"}}),
    ))
    .expect("send target");

    let output = collect_turn_output_from_live_with_limits(
        &mut rx,
        "thr_target",
        "turn_target",
        1,
        Duration::from_secs(1),
    )
    .await
    .expect("must collect output");

    assert_eq!(output["status"], "ok");
}
