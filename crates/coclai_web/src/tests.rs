use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use coclai_runtime::api::ThreadStartParams;
use coclai_runtime::approvals::ServerRequest;
use coclai_runtime::events::{Direction, MsgKind};
use coclai_runtime::runtime::{RuntimeConfig, SchemaGuardConfig};
use coclai_runtime::transport::StdioProcessSpec;
use coclai_runtime::PluginContractVersion;
use serde_json::json;
use tokio::time::{sleep, timeout, Instant};

use super::*;

fn workspace_schema_guard() -> SchemaGuardConfig {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let active = manifest_dir.join("../../SCHEMAS/app-server/active");
    SchemaGuardConfig {
        active_schema_dir: active,
    }
}

fn python_web_mock_process() -> StdioProcessSpec {
    let script = r#"
import json
import sys

approval_threads = {}

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
    params = msg.get("params") or {}

    # Client response to server request: mirror as ack notification for assertions.
    if method is None and rpc_id is not None and ("result" in msg or "error" in msg):
        thread_id = approval_threads.get(rpc_id, "thr_unknown")
        sys.stdout.write(json.dumps({
            "method": "approval/ack",
            "params": {
                "threadId": thread_id,
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

    if rpc_id is None:
        continue

    if method == "thread/start":
        thread_id = f"thr_{rpc_id}"
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"thread": {"id": thread_id}}}) + "\n")
        sys.stdout.write(json.dumps({"method":"thread/started","params":{"thread":{"id":thread_id}}}) + "\n")
        sys.stdout.flush()
        continue

    if method == "thread/resume":
        thread_id = params.get("threadId", "thr_resume")
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"thread": {"id": thread_id}}}) + "\n")
        sys.stdout.write(json.dumps({"method":"thread/started","params":{"thread":{"id":thread_id}}}) + "\n")
        sys.stdout.flush()
        continue

    if method == "turn/start":
        thread_id = params.get("threadId", "thr_missing")
        turn_id = f"turn_{rpc_id}"
        input_items = params.get("input") or []
        first_text = ""
        if len(input_items) > 0 and isinstance(input_items[0], dict):
            first_text = input_items[0].get("text", "")

        sys.stdout.write(json.dumps({"method":"turn/started","params":{"threadId":thread_id,"turnId":turn_id}}) + "\n")
        if first_text == "need_approval":
            approval_id = 800 + int(rpc_id)
            approval_threads[approval_id] = thread_id
            sys.stdout.write(json.dumps({
                "id": approval_id,
                "method": "item/fileChange/requestApproval",
                "params": {"threadId": thread_id, "turnId": turn_id, "itemId": "item_1"}
            }) + "\n")
        sys.stdout.write(json.dumps({"method":"turn/completed","params":{"threadId":thread_id,"turnId":turn_id}}) + "\n")
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"turn": {"id": turn_id}}}) + "\n")
        sys.stdout.flush()
        continue

    if method == "turn/interrupt":
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"ok": True}}) + "\n")
        sys.stdout.flush()
        continue

    sys.stdout.write(json.dumps({"id": rpc_id, "result": {"echoMethod": method}}) + "\n")
    sys.stdout.flush()
"#;

    let mut spec = StdioProcessSpec::new("python3");
    spec.args = vec!["-u".to_owned(), "-c".to_owned(), script.to_owned()];
    spec
}

async fn spawn_mock_runtime() -> Runtime {
    let cfg = RuntimeConfig::new(python_web_mock_process(), workspace_schema_guard());
    Runtime::spawn_local(cfg).await.expect("runtime spawn")
}

fn turn_task(text: &str) -> Value {
    json!({
        "input": [{ "type": "text", "text": text }],
        "approvalPolicy": "never",
        "sandboxPolicy": { "type": "readOnly" },
        "outputSchema": {
            "type": "object",
            "required": ["status"],
            "properties": { "status": { "type": "string" } }
        }
    })
}

#[derive(Clone)]
struct FakeWebAdapter {
    state: Arc<Mutex<FakeWebAdapterState>>,
    streams: Arc<Mutex<Option<WebRuntimeStreams>>>,
}

#[derive(Debug)]
struct FakeWebAdapterState {
    start_thread_id: String,
    turn_start_result: Value,
    start_calls: usize,
    start_params: Vec<ThreadStartParams>,
    resume_calls: Vec<(String, ThreadStartParams)>,
    turn_start_calls: Vec<Value>,
    archive_calls: Vec<String>,
    approval_calls: Vec<(String, Value)>,
    pending_approval_ids: Vec<String>,
    take_stream_calls: usize,
}

impl Default for FakeWebAdapterState {
    fn default() -> Self {
        Self {
            start_thread_id: "thr_fake_web".to_owned(),
            turn_start_result: json!({"turn":{"id":"turn_fake_web"}}),
            start_calls: 0,
            start_params: Vec::new(),
            resume_calls: Vec::new(),
            turn_start_calls: Vec::new(),
            archive_calls: Vec::new(),
            approval_calls: Vec::new(),
            pending_approval_ids: Vec::new(),
            take_stream_calls: 0,
        }
    }
}

impl WebPluginAdapter for FakeWebAdapter {
    fn take_streams<'a>(&'a self) -> WebAdapterFuture<'a, Result<WebRuntimeStreams, WebError>> {
        Box::pin(async move {
            let mut state = self.state.lock().expect("fake adapter state lock");
            state.take_stream_calls += 1;
            drop(state);
            let mut streams = self.streams.lock().expect("fake adapter stream lock");
            streams.take().ok_or(WebError::AlreadyBound)
        })
    }

    fn thread_start<'a>(
        &'a self,
        params: ThreadStartParams,
    ) -> WebAdapterFuture<'a, Result<String, WebError>> {
        Box::pin(async move {
            let mut state = self.state.lock().expect("fake adapter state lock");
            state.start_calls += 1;
            state.start_params.push(params);
            Ok(state.start_thread_id.clone())
        })
    }

    fn thread_resume<'a>(
        &'a self,
        thread_id: &'a str,
        params: ThreadStartParams,
    ) -> WebAdapterFuture<'a, Result<String, WebError>> {
        Box::pin(async move {
            let mut state = self.state.lock().expect("fake adapter state lock");
            state.resume_calls.push((thread_id.to_owned(), params));
            Ok(thread_id.to_owned())
        })
    }

    fn turn_start<'a>(
        &'a self,
        turn_params: Value,
    ) -> WebAdapterFuture<'a, Result<Value, WebError>> {
        Box::pin(async move {
            let mut state = self.state.lock().expect("fake adapter state lock");
            state.turn_start_calls.push(turn_params);
            Ok(state.turn_start_result.clone())
        })
    }

    fn thread_archive<'a>(
        &'a self,
        thread_id: &'a str,
    ) -> WebAdapterFuture<'a, Result<(), WebError>> {
        Box::pin(async move {
            let mut state = self.state.lock().expect("fake adapter state lock");
            state.archive_calls.push(thread_id.to_owned());
            Ok(())
        })
    }

    fn respond_approval_ok<'a>(
        &'a self,
        approval_id: &'a str,
        result: Value,
    ) -> WebAdapterFuture<'a, Result<(), WebError>> {
        Box::pin(async move {
            let mut state = self.state.lock().expect("fake adapter state lock");
            state.approval_calls.push((approval_id.to_owned(), result));
            Ok(())
        })
    }

    fn pending_approval_ids(&self) -> Vec<String> {
        self.state
            .lock()
            .expect("fake adapter state lock")
            .pending_approval_ids
            .clone()
    }
}

#[derive(Clone)]
struct IncompatibleWebAdapter;

impl WebPluginAdapter for IncompatibleWebAdapter {
    fn plugin_contract_version(&self) -> PluginContractVersion {
        PluginContractVersion::new(2, 0)
    }

    fn take_streams<'a>(&'a self) -> WebAdapterFuture<'a, Result<WebRuntimeStreams, WebError>> {
        Box::pin(async move { panic!("take_streams must not run on incompatible adapter") })
    }

    fn thread_start<'a>(
        &'a self,
        _params: ThreadStartParams,
    ) -> WebAdapterFuture<'a, Result<String, WebError>> {
        Box::pin(async move { panic!("thread_start must not run on incompatible adapter") })
    }

    fn thread_resume<'a>(
        &'a self,
        _thread_id: &'a str,
        _params: ThreadStartParams,
    ) -> WebAdapterFuture<'a, Result<String, WebError>> {
        Box::pin(async move { panic!("thread_resume must not run on incompatible adapter") })
    }

    fn turn_start<'a>(
        &'a self,
        _turn_params: Value,
    ) -> WebAdapterFuture<'a, Result<Value, WebError>> {
        Box::pin(async move { panic!("turn_start must not run on incompatible adapter") })
    }

    fn thread_archive<'a>(
        &'a self,
        _thread_id: &'a str,
    ) -> WebAdapterFuture<'a, Result<(), WebError>> {
        Box::pin(async move { panic!("thread_archive must not run on incompatible adapter") })
    }

    fn respond_approval_ok<'a>(
        &'a self,
        _approval_id: &'a str,
        _result: Value,
    ) -> WebAdapterFuture<'a, Result<(), WebError>> {
        Box::pin(async move { panic!("respond_approval_ok must not run on incompatible adapter") })
    }

    fn pending_approval_ids(&self) -> Vec<String> {
        Vec::new()
    }
}

#[derive(Clone)]
struct CompatibleMinorWebAdapter;

impl WebPluginAdapter for CompatibleMinorWebAdapter {
    fn plugin_contract_version(&self) -> PluginContractVersion {
        PluginContractVersion::new(1, 42)
    }

    fn take_streams<'a>(&'a self) -> WebAdapterFuture<'a, Result<WebRuntimeStreams, WebError>> {
        Box::pin(async move {
            let (_request_tx, request_rx) = tokio::sync::mpsc::channel::<ServerRequest>(8);
            let (_live_tx, live_rx) = broadcast::channel::<Envelope>(8);
            Ok(WebRuntimeStreams {
                request_rx,
                live_rx,
            })
        })
    }

    fn thread_start<'a>(
        &'a self,
        _params: ThreadStartParams,
    ) -> WebAdapterFuture<'a, Result<String, WebError>> {
        Box::pin(async move { panic!("thread_start is not expected in compatibility-spawn test") })
    }

    fn thread_resume<'a>(
        &'a self,
        _thread_id: &'a str,
        _params: ThreadStartParams,
    ) -> WebAdapterFuture<'a, Result<String, WebError>> {
        Box::pin(async move { panic!("thread_resume is not expected in compatibility-spawn test") })
    }

    fn turn_start<'a>(
        &'a self,
        _turn_params: Value,
    ) -> WebAdapterFuture<'a, Result<Value, WebError>> {
        Box::pin(async move { panic!("turn_start is not expected in compatibility-spawn test") })
    }

    fn thread_archive<'a>(
        &'a self,
        _thread_id: &'a str,
    ) -> WebAdapterFuture<'a, Result<(), WebError>> {
        Box::pin(
            async move { panic!("thread_archive is not expected in compatibility-spawn test") },
        )
    }

    fn respond_approval_ok<'a>(
        &'a self,
        _approval_id: &'a str,
        _result: Value,
    ) -> WebAdapterFuture<'a, Result<(), WebError>> {
        Box::pin(async move {
            panic!("respond_approval_ok is not expected in compatibility-spawn test")
        })
    }

    fn pending_approval_ids(&self) -> Vec<String> {
        Vec::new()
    }
}

async fn wait_turn_completed(rx: &mut broadcast::Receiver<Envelope>, thread_id: &str) -> Envelope {
    loop {
        let envelope = timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("event timeout")
            .expect("event channel closed");
        if envelope.thread_id.as_deref() == Some(thread_id)
            && envelope.method.as_deref() == Some("turn/completed")
        {
            return envelope;
        }
    }
}

async fn assert_no_thread_leak(
    rx: &mut broadcast::Receiver<Envelope>,
    thread_id: &str,
    duration: Duration,
) {
    let deadline = Instant::now() + duration;
    loop {
        let now = Instant::now();
        if now >= deadline {
            break;
        }
        let remaining = deadline.duration_since(now);
        let poll = remaining.min(Duration::from_millis(40));
        match timeout(poll, rx.recv()).await {
            Ok(Ok(envelope)) => {
                if envelope.thread_id.as_deref() == Some(thread_id) {
                    panic!("cross-session leak detected for thread {thread_id}");
                }
            }
            Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
            Ok(Err(broadcast::error::RecvError::Closed)) => break,
            Err(_) => sleep(Duration::from_millis(5)).await,
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn serialize_envelope_to_sse() {
    let envelope = Envelope {
        seq: 1,
        ts_millis: 0,
        direction: Direction::Inbound,
        kind: MsgKind::Notification,
        rpc_id: None,
        method: Some("turn/started".to_owned()),
        thread_id: Some("thr_1".to_owned()),
        turn_id: Some("turn_1".to_owned()),
        item_id: None,
        json: json!({"method":"turn/started","params":{}}),
    };

    let sse = serialize_sse_envelope(&envelope).expect("serialize");
    assert!(sse.starts_with("data: {"));
    assert!(sse.ends_with("\n\n"));
}

#[tokio::test(flavor = "current_thread")]
async fn spawn_with_adapter_rejects_incompatible_contract_version() {
    let adapter: Arc<dyn WebPluginAdapter> = Arc::new(IncompatibleWebAdapter);
    let err = match WebAdapter::spawn_with_adapter(adapter, WebAdapterConfig::default()).await {
        Ok(_) => panic!("must reject mismatch"),
        Err(err) => err,
    };
    assert_eq!(
        err,
        WebError::IncompatibleContract {
            expected_major: 1,
            expected_minor: 0,
            actual_major: 2,
            actual_minor: 0,
        }
    );
}

#[tokio::test(flavor = "current_thread")]
async fn spawn_with_adapter_accepts_compatible_minor_contract_version() {
    let adapter: Arc<dyn WebPluginAdapter> = Arc::new(CompatibleMinorWebAdapter);
    let web = WebAdapter::spawn_with_adapter(adapter, WebAdapterConfig::default())
        .await
        .expect("minor version must remain compatible");
    drop(web);
}

#[tokio::test(flavor = "current_thread")]
async fn web_adapter_uses_plugin_boundary_without_runtime_dependency() {
    let (_live_tx, live_rx) = broadcast::channel::<Envelope>(8);
    let (_request_tx, request_rx) = tokio::sync::mpsc::channel::<ServerRequest>(8);
    let state = Arc::new(Mutex::new(FakeWebAdapterState::default()));
    let adapter: Arc<dyn WebPluginAdapter> = Arc::new(FakeWebAdapter {
        state: Arc::clone(&state),
        streams: Arc::new(Mutex::new(Some(WebRuntimeStreams {
            request_rx,
            live_rx,
        }))),
    });

    let web = WebAdapter::spawn_with_adapter(adapter, WebAdapterConfig::default())
        .await
        .expect("spawn with fake adapter");

    let session = web
        .create_session(
            "tenant_a",
            CreateSessionRequest {
                artifact_id: "doc:web-adapter".to_owned(),
                model: Some("gpt-fake".to_owned()),
                thread_id: None,
            },
        )
        .await
        .expect("create session");
    assert_eq!(session.thread_id, "thr_fake_web");

    let turn = web
        .create_turn(
            "tenant_a",
            &session.session_id,
            CreateTurnRequest {
                task: turn_task("hello"),
            },
        )
        .await
        .expect("create turn");
    assert_eq!(turn.turn_id, "turn_fake_web");

    let closed = web
        .close_session("tenant_a", &session.session_id)
        .await
        .expect("close session");
    assert_eq!(closed.thread_id, "thr_fake_web");
    assert!(closed.archived);

    let state = state.lock().expect("fake adapter state lock");
    assert_eq!(state.take_stream_calls, 1);
    assert_eq!(state.start_calls, 1);
    assert_eq!(state.turn_start_calls.len(), 1);
    assert_eq!(state.turn_start_calls[0]["threadId"], "thr_fake_web");
    assert_eq!(state.archive_calls, vec!["thr_fake_web".to_owned()]);
}

#[tokio::test(flavor = "current_thread")]
async fn spawn_rejects_zero_capacity_config() {
    let runtime = spawn_mock_runtime().await;

    let err = match WebAdapter::spawn(
        runtime.clone(),
        WebAdapterConfig {
            session_event_channel_capacity: 0,
            session_approval_channel_capacity: 128,
        },
    )
    .await
    {
        Ok(_) => panic!("must reject zero event capacity"),
        Err(err) => err,
    };
    assert!(matches!(err, WebError::InvalidConfig(_)));

    let err = match WebAdapter::spawn(
        runtime.clone(),
        WebAdapterConfig {
            session_event_channel_capacity: 128,
            session_approval_channel_capacity: 0,
        },
    )
    .await
    {
        Ok(_) => panic!("must reject zero approval capacity"),
        Err(err) => err,
    };
    assert!(matches!(err, WebError::InvalidConfig(_)));

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn spawn_rejects_second_adapter_on_same_runtime() {
    let runtime = spawn_mock_runtime().await;
    let _adapter = WebAdapter::spawn(runtime.clone(), WebAdapterConfig::default())
        .await
        .expect("first adapter spawn");

    let err = match WebAdapter::spawn(runtime.clone(), WebAdapterConfig::default()).await {
        Ok(_) => panic!("second adapter on same runtime must fail"),
        Err(err) => err,
    };
    assert_eq!(err, WebError::AlreadyBound);

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn sessions_turns_and_events_are_isolated() {
    let runtime = spawn_mock_runtime().await;
    let adapter = WebAdapter::spawn(runtime.clone(), WebAdapterConfig::default())
        .await
        .expect("adapter spawn");

    let session_a = adapter
        .create_session(
            "tenant_a",
            CreateSessionRequest {
                artifact_id: "doc:a".to_owned(),
                model: None,
                thread_id: None,
            },
        )
        .await
        .expect("create session a");
    let session_b = adapter
        .create_session(
            "tenant_a",
            CreateSessionRequest {
                artifact_id: "doc:b".to_owned(),
                model: None,
                thread_id: None,
            },
        )
        .await
        .expect("create session b");
    assert_ne!(session_a.thread_id, session_b.thread_id);

    let mut events_a = adapter
        .subscribe_session_events("tenant_a", &session_a.session_id)
        .await
        .expect("events a");

    adapter
        .create_turn(
            "tenant_a",
            &session_a.session_id,
            CreateTurnRequest {
                task: turn_task("hello-a"),
            },
        )
        .await
        .expect("turn a");
    let completed_a = wait_turn_completed(&mut events_a, &session_a.thread_id).await;
    assert_eq!(
        completed_a.thread_id.as_deref(),
        Some(session_a.thread_id.as_str())
    );

    adapter
        .create_turn(
            "tenant_a",
            &session_b.session_id,
            CreateTurnRequest {
                task: turn_task("hello-b"),
            },
        )
        .await
        .expect("turn b");
    assert_no_thread_leak(
        &mut events_a,
        &session_b.thread_id,
        Duration::from_millis(250),
    )
    .await;

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn tenant_isolation_blocks_cross_access() {
    let runtime = spawn_mock_runtime().await;
    let adapter = WebAdapter::spawn(runtime.clone(), WebAdapterConfig::default())
        .await
        .expect("adapter spawn");

    let session = adapter
        .create_session(
            "tenant_a",
            CreateSessionRequest {
                artifact_id: "doc:a".to_owned(),
                model: None,
                thread_id: None,
            },
        )
        .await
        .expect("create session");

    let err = adapter
        .create_turn(
            "tenant_b",
            &session.session_id,
            CreateTurnRequest {
                task: turn_task("hello"),
            },
        )
        .await
        .expect_err("must block cross-tenant turn");
    assert_eq!(err, WebError::Forbidden);

    let err = adapter
        .subscribe_session_events("tenant_b", &session.session_id)
        .await
        .expect_err("must block cross-tenant event subscribe");
    assert_eq!(err, WebError::Forbidden);

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn create_session_rejects_untracked_thread_id() {
    let runtime = spawn_mock_runtime().await;
    let adapter = WebAdapter::spawn(runtime.clone(), WebAdapterConfig::default())
        .await
        .expect("adapter spawn");

    let err = adapter
        .create_session(
            "tenant_a",
            CreateSessionRequest {
                artifact_id: "doc:resume".to_owned(),
                model: None,
                thread_id: Some("thr_untracked".to_owned()),
            },
        )
        .await
        .expect_err("untracked thread id must be rejected");
    assert_eq!(err, WebError::Forbidden);

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn close_session_removes_session_indexes() {
    let runtime = spawn_mock_runtime().await;
    let adapter = WebAdapter::spawn(runtime.clone(), WebAdapterConfig::default())
        .await
        .expect("adapter spawn");

    let session = adapter
        .create_session(
            "tenant_a",
            CreateSessionRequest {
                artifact_id: "doc:close".to_owned(),
                model: None,
                thread_id: None,
            },
        )
        .await
        .expect("create session");

    let closed = adapter
        .close_session("tenant_a", &session.session_id)
        .await
        .expect("close session");
    assert_eq!(closed.thread_id, session.thread_id);
    assert!(closed.archived);

    let err = adapter
        .subscribe_session_events("tenant_a", &session.session_id)
        .await
        .expect_err("session must be removed");
    assert_eq!(err, WebError::InvalidSession);

    let err = adapter
        .create_turn(
            "tenant_a",
            &session.session_id,
            CreateTurnRequest {
                task: turn_task("after-close"),
            },
        )
        .await
        .expect_err("closed session turn must fail");
    assert_eq!(err, WebError::InvalidSession);

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn close_session_keeps_local_cleanup_when_archive_fails() {
    let runtime = spawn_mock_runtime().await;
    let adapter = WebAdapter::spawn(runtime.clone(), WebAdapterConfig::default())
        .await
        .expect("adapter spawn");

    let session = adapter
        .create_session(
            "tenant_a",
            CreateSessionRequest {
                artifact_id: "doc:close-fail".to_owned(),
                model: None,
                thread_id: None,
            },
        )
        .await
        .expect("create session");

    runtime.shutdown().await.expect("shutdown runtime first");

    let closed = adapter
        .close_session("tenant_a", &session.session_id)
        .await
        .expect("close session must still clean local state");
    assert_eq!(closed.thread_id, session.thread_id);
    assert!(!closed.archived);

    let err = adapter
        .subscribe_session_events("tenant_a", &session.session_id)
        .await
        .expect_err("session must be removed even when archive fails");
    assert_eq!(err, WebError::InvalidSession);
}

#[tokio::test(flavor = "current_thread")]
async fn approval_roundtrip_via_post_approval() {
    let runtime = spawn_mock_runtime().await;
    let adapter = WebAdapter::spawn(runtime.clone(), WebAdapterConfig::default())
        .await
        .expect("adapter spawn");

    let session = adapter
        .create_session(
            "tenant_a",
            CreateSessionRequest {
                artifact_id: "doc:approval".to_owned(),
                model: None,
                thread_id: None,
            },
        )
        .await
        .expect("create session");

    let mut approvals = adapter
        .subscribe_session_approvals("tenant_a", &session.session_id)
        .await
        .expect("subscribe approvals");
    let mut events = adapter
        .subscribe_session_events("tenant_a", &session.session_id)
        .await
        .expect("subscribe events");

    adapter
        .create_turn(
            "tenant_a",
            &session.session_id,
            CreateTurnRequest {
                task: turn_task("need_approval"),
            },
        )
        .await
        .expect("create turn");

    let request = timeout(Duration::from_secs(2), approvals.recv())
        .await
        .expect("approval timeout")
        .expect("approval channel closed");
    assert_eq!(request.method, "item/fileChange/requestApproval");

    adapter
        .post_approval(
            "tenant_a",
            &session.session_id,
            &request.approval_id,
            ApprovalResponsePayload {
                decision: Some(Value::String("decline".to_owned())),
                result: None,
            },
        )
        .await
        .expect("post approval");

    loop {
        let envelope = timeout(Duration::from_secs(2), events.recv())
            .await
            .expect("ack timeout")
            .expect("event channel closed");
        if envelope.method.as_deref() == Some("approval/ack") {
            assert_eq!(
                envelope.thread_id.as_deref(),
                Some(session.thread_id.as_str())
            );
            break;
        }
    }

    runtime.shutdown().await.expect("shutdown");
}
