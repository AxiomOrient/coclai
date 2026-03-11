use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::plugin::PluginContractVersion;
use crate::runtime::api::ThreadStartParams;
use crate::runtime::approvals::ServerRequest;
use crate::runtime::core::RuntimeConfig;
use crate::runtime::events::{Direction, JsonRpcId, MsgKind};
use crate::runtime::transport::StdioProcessSpec;
use serde_json::json;
use serde_json::Value;
use tokio::time::{sleep, timeout, Instant};

use super::*;

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

    crate::test_fixtures::python_inline_process(script)
}

async fn spawn_mock_runtime() -> Runtime {
    let cfg = RuntimeConfig::new(python_web_mock_process());
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
    resume_result_thread_id: Option<String>,
    turn_start_calls: Vec<Value>,
    archive_calls: Vec<String>,
    archive_failures_remaining: usize,
    archive_block_on: Option<Arc<tokio::sync::Notify>>,
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
            resume_result_thread_id: None,
            turn_start_calls: Vec::new(),
            archive_calls: Vec::new(),
            archive_failures_remaining: 0,
            archive_block_on: None,
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
            Ok(state
                .resume_result_thread_id
                .clone()
                .unwrap_or_else(|| thread_id.to_owned()))
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
            let (block_on, should_fail) = {
                let mut state = self.state.lock().expect("fake adapter state lock");
                state.archive_calls.push(thread_id.to_owned());
                let block_on = state.archive_block_on.clone();
                let should_fail = if state.archive_failures_remaining > 0 {
                    state.archive_failures_remaining -= 1;
                    true
                } else {
                    false
                };
                (block_on, should_fail)
            };

            if let Some(gate) = block_on {
                gate.notified().await;
            }

            if should_fail {
                return Err(WebError::Internal("forced archive failure".to_owned()));
            }
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

// Unit: pure serialization/parsing boundaries.
mod serialization;
// Contract: ownership/isolation and adapter contract guarantees.
mod approval_boundaries;
mod contract_and_spawn;
// Integration: runtime-backed flow behavior.
mod approvals;
mod routing_observability;
mod session_flows;
