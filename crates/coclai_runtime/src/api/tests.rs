use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use coclai_plugin_core::{
    HookAction, HookContext, HookIssue, HookIssueClass, HookPhase, PostHook, PreHook,
};

use crate::{RuntimeConfig, RuntimeHookConfig, SchemaGuardConfig, StdioProcessSpec};
use serde_json::{json, Value};

use super::*;

fn workspace_schema_guard() -> SchemaGuardConfig {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let active = manifest_dir.join("../../SCHEMAS/app-server/active");
    SchemaGuardConfig {
        active_schema_dir: active,
    }
}

fn python_api_mock_process() -> StdioProcessSpec {
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
    params = msg.get("params") or {}

    def make_thread(thread_id):
        return {
            "id": thread_id,
            "cliVersion": "0.104.0",
            "createdAt": 1700000000,
            "cwd": "/tmp",
            "modelProvider": "openai",
            "path": f"/tmp/threads/{thread_id}.jsonl",
            "preview": "hello",
            "source": "app-server",
            "turns": [],
            "updatedAt": 1700000001,
        }

    def make_turn(turn_id, status, items):
        return {
            "id": turn_id,
            "status": status,
            "items": items,
        }

    if method == "initialize" and rpc_id is not None:
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"ready": True}}) + "\n")
        sys.stdout.flush()
        continue

    if rpc_id is None:
        continue

    if method == "thread/start":
        out = {"id": rpc_id, "result": {"thread": {"id": "thr_typed"}}}
    elif method == "thread/resume":
        out = {"id": rpc_id, "result": {"threadId": params.get("threadId", "thr_resume")}}
    elif method == "thread/fork":
        out = {"id": rpc_id, "result": {"id": "thr_forked"}}
    elif method == "thread/archive":
        out = {"id": rpc_id, "result": {"ok": True, "threadId": params.get("threadId")}}
    elif method == "thread/read":
        thread = make_thread(params.get("threadId", "thr_read"))
        thread["turnsIncluded"] = bool(params.get("includeTurns"))
        if params.get("includeTurns"):
            thread["turns"] = [
                make_turn(
                    "turn_read_1",
                    "completed",
                    [{"id": "item_read_1", "type": "agentMessage", "text": "ok"}],
                )
            ]
        out = {
            "id": rpc_id,
            "result": {
                "thread": thread
            },
        }
    elif method == "thread/list":
        thread = make_thread("thr_list")
        thread["archivedFilter"] = params.get("archived")
        thread["sortKey"] = params.get("sortKey")
        thread["providerCount"] = len(params.get("modelProviders") or [])
        out = {
            "id": rpc_id,
            "result": {
                "data": [thread],
                "nextCursor": params.get("cursor"),
            },
        }
    elif method == "thread/loaded/list":
        limit = params.get("limit")
        data = ["thr_loaded_1", "thr_loaded_2"] if limit is None else [f"thr_loaded_{limit}"]
        out = {
            "id": rpc_id,
            "result": {"data": data, "nextCursor": params.get("cursor")},
        }
    elif method == "thread/rollback":
        thread = make_thread(params.get("threadId", "thr_rolled"))
        thread["rolledBackTurns"] = params.get("numTurns")
        thread["turns"] = [
            make_turn(
                "turn_rollback_1",
                "failed",
                [
                    {
                        "id": "item_rollback_1",
                        "type": "commandExecution",
                        "command": "false",
                        "commandActions": [],
                        "cwd": "/tmp",
                        "status": "failed",
                    }
                ],
            )
        ]
        out = {
            "id": rpc_id,
            "result": {
                "thread": thread
            },
        }
    elif method == "turn/start":
        out = {"id": rpc_id, "result": {"turn": {"id": "turn_typed"}, "echoParams": params}}
    elif method == "turn/interrupt":
        out = {"id": rpc_id, "result": {"ok": True, "turnId": params.get("turnId")}}
    else:
        out = {"id": rpc_id, "result": {"echoMethod": method, "params": params}}

    sys.stdout.write(json.dumps(out) + "\n")
    sys.stdout.flush()
"#;

    let mut spec = StdioProcessSpec::new("python3");
    spec.args = vec!["-u".to_owned(), "-c".to_owned(), script.to_owned()];
    spec
}

fn python_run_prompt_mock_process() -> StdioProcessSpec {
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
    params = msg.get("params") or {}

    if method == "initialize" and rpc_id is not None:
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"ready": True}}) + "\n")
        sys.stdout.flush()
        continue

    if rpc_id is None:
        continue

    if method == "thread/start":
        thread_id = "thr_prompt"
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"thread": {"id": thread_id}}}) + "\n")
        sys.stdout.write(json.dumps({"method":"thread/started","params":{"threadId":thread_id}}) + "\n")
        sys.stdout.flush()
        continue

    if method == "thread/resume":
        thread_id = params.get("threadId", "thr_prompt")
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"threadId": thread_id}}) + "\n")
        sys.stdout.write(json.dumps({"method":"thread/started","params":{"threadId":thread_id}}) + "\n")
        sys.stdout.flush()
        continue

    if method == "turn/start":
        thread_id = params.get("threadId", "thr_prompt")
        turn_id = "turn_prompt"
        sys.stdout.write(json.dumps({"method":"turn/started","params":{"threadId":thread_id,"turnId":turn_id}}) + "\n")
        sys.stdout.write(json.dumps({"method":"item/started","params":{"threadId":thread_id,"turnId":turn_id,"itemId":"item_prompt","itemType":"agentMessage"}}) + "\n")
        sys.stdout.write(json.dumps({"method":"item/agentMessage/delta","params":{"threadId":thread_id,"turnId":turn_id,"itemId":"item_prompt","delta":"ok-from-run-prompt"}}) + "\n")
        sys.stdout.write(json.dumps({"method":"item/completed","params":{"threadId":thread_id,"turnId":turn_id,"itemId":"item_prompt","item":{"type":"agent_message","text":"ok-from-run-prompt"}}}) + "\n")
        sys.stdout.write(json.dumps({"method":"turn/completed","params":{"threadId":thread_id,"turnId":turn_id}}) + "\n")
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"turn": {"id": turn_id}}}) + "\n")
        sys.stdout.flush()
        continue

    sys.stdout.write(json.dumps({"id": rpc_id, "result": {"echoMethod": method, "params": params}}) + "\n")
    sys.stdout.flush()
"#;

    let mut spec = StdioProcessSpec::new("python3");
    spec.args = vec!["-u".to_owned(), "-c".to_owned(), script.to_owned()];
    spec
}

fn python_run_prompt_cross_thread_noise_process() -> StdioProcessSpec {
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
    params = msg.get("params") or {}

    if method == "initialize" and rpc_id is not None:
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"ready": True}}) + "\n")
        sys.stdout.flush()
        continue

    if rpc_id is None:
        continue

    if method == "thread/start":
        thread_id = "thr_prompt"
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"thread": {"id": thread_id}}}) + "\n")
        sys.stdout.write(json.dumps({"method":"thread/started","params":{"threadId":thread_id}}) + "\n")
        sys.stdout.flush()
        continue

    if method == "turn/start":
        thread_id = params.get("threadId", "thr_prompt")
        turn_id = "turn_prompt"

        # Cross-thread noise with same turn id; client must ignore this.
        sys.stdout.write(json.dumps({"method":"turn/started","params":{"threadId":"thr_other","turnId":turn_id}}) + "\n")
        sys.stdout.write(json.dumps({"method":"item/started","params":{"threadId":"thr_other","turnId":turn_id,"itemId":"item_noise","itemType":"agentMessage"}}) + "\n")
        sys.stdout.write(json.dumps({"method":"item/agentMessage/delta","params":{"threadId":"thr_other","turnId":turn_id,"itemId":"item_noise","delta":"wrong-thread"}}) + "\n")
        sys.stdout.write(json.dumps({"method":"turn/completed","params":{"threadId":"thr_other","turnId":turn_id}}) + "\n")

        sys.stdout.write(json.dumps({"method":"turn/started","params":{"threadId":thread_id,"turnId":turn_id}}) + "\n")
        sys.stdout.write(json.dumps({"method":"item/started","params":{"threadId":thread_id,"turnId":turn_id,"itemId":"item_prompt","itemType":"agentMessage"}}) + "\n")
        sys.stdout.write(json.dumps({"method":"item/agentMessage/delta","params":{"threadId":thread_id,"turnId":turn_id,"itemId":"item_prompt","delta":"ok-from-run-prompt"}}) + "\n")
        sys.stdout.write(json.dumps({"method":"item/completed","params":{"threadId":thread_id,"turnId":turn_id,"itemId":"item_prompt","item":{"type":"agent_message","text":"ok-from-run-prompt"}}}) + "\n")
        sys.stdout.write(json.dumps({"method":"turn/completed","params":{"threadId":thread_id,"turnId":turn_id}}) + "\n")
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"turn": {"id": turn_id}}}) + "\n")
        sys.stdout.flush()
        continue

    sys.stdout.write(json.dumps({"id": rpc_id, "result": {"echoMethod": method, "params": params}}) + "\n")
    sys.stdout.flush()
"#;

    let mut spec = StdioProcessSpec::new("python3");
    spec.args = vec!["-u".to_owned(), "-c".to_owned(), script.to_owned()];
    spec
}

fn python_run_prompt_error_mock_process() -> StdioProcessSpec {
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
    params = msg.get("params") or {}

    if method == "initialize" and rpc_id is not None:
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"ready": True}}) + "\n")
        sys.stdout.flush()
        continue

    if rpc_id is None:
        continue

    if method == "thread/start":
        thread_id = "thr_prompt_err"
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"thread": {"id": thread_id}}}) + "\n")
        sys.stdout.write(json.dumps({"method":"thread/started","params":{"threadId":thread_id}}) + "\n")
        sys.stdout.flush()
        continue

    if method == "turn/start":
        thread_id = params.get("threadId", "thr_prompt_err")
        turn_id = "turn_prompt_err"
        sys.stdout.write(json.dumps({"method":"turn/started","params":{"threadId":thread_id,"turnId":turn_id}}) + "\n")
        sys.stdout.write(json.dumps({"method":"error","params":{"threadId":thread_id,"turnId":turn_id,"message":"model unavailable"}}) + "\n")
        sys.stdout.write(json.dumps({"method":"turn/completed","params":{"threadId":thread_id,"turnId":turn_id}}) + "\n")
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"turn": {"id": turn_id}}}) + "\n")
        sys.stdout.flush()
        continue

    sys.stdout.write(json.dumps({"id": rpc_id, "result": {"echoMethod": method, "params": params}}) + "\n")
    sys.stdout.flush()
"#;

    let mut spec = StdioProcessSpec::new("python3");
    spec.args = vec!["-u".to_owned(), "-c".to_owned(), script.to_owned()];
    spec
}

fn python_run_prompt_turn_failed_mock_process() -> StdioProcessSpec {
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
    params = msg.get("params") or {}

    if method == "initialize" and rpc_id is not None:
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"ready": True}}) + "\n")
        sys.stdout.flush()
        continue

    if rpc_id is None:
        continue

    if method == "thread/start":
        thread_id = "thr_prompt_fail"
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"thread": {"id": thread_id}}}) + "\n")
        sys.stdout.write(json.dumps({"method":"thread/started","params":{"threadId":thread_id}}) + "\n")
        sys.stdout.flush()
        continue

    if method == "turn/start":
        thread_id = params.get("threadId", "thr_prompt_fail")
        turn_id = "turn_prompt_fail"
        sys.stdout.write(json.dumps({"method":"turn/started","params":{"threadId":thread_id,"turnId":turn_id}}) + "\n")
        sys.stdout.write(json.dumps({"method":"turn/failed","params":{"threadId":thread_id,"turnId":turn_id,"error":{"code":429,"message":"rate limited"}}}) + "\n")
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"turn": {"id": turn_id}}}) + "\n")
        sys.stdout.flush()
        continue

    sys.stdout.write(json.dumps({"id": rpc_id, "result": {"echoMethod": method, "params": params}}) + "\n")
    sys.stdout.flush()
"#;

    let mut spec = StdioProcessSpec::new("python3");
    spec.args = vec!["-u".to_owned(), "-c".to_owned(), script.to_owned()];
    spec
}

fn python_run_prompt_effort_probe_process() -> StdioProcessSpec {
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
    params = msg.get("params") or {}

    if method == "initialize" and rpc_id is not None:
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"ready": True}}) + "\n")
        sys.stdout.flush()
        continue

    if rpc_id is None:
        continue

    if method == "thread/start":
        thread_id = "thr_effort_probe"
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"thread": {"id": thread_id}}}) + "\n")
        sys.stdout.write(json.dumps({"method":"thread/started","params":{"threadId":thread_id}}) + "\n")
        sys.stdout.flush()
        continue

    if method == "turn/start":
        thread_id = params.get("threadId", "thr_effort_probe")
        turn_id = "turn_effort_probe"
        effort = params.get("effort", "missing")
        sys.stdout.write(json.dumps({"method":"turn/started","params":{"threadId":thread_id,"turnId":turn_id}}) + "\n")
        sys.stdout.write(json.dumps({"method":"item/started","params":{"threadId":thread_id,"turnId":turn_id,"itemId":"item_effort_probe","itemType":"agentMessage"}}) + "\n")
        sys.stdout.write(json.dumps({"method":"item/agentMessage/delta","params":{"threadId":thread_id,"turnId":turn_id,"itemId":"item_effort_probe","delta":str(effort)}}) + "\n")
        sys.stdout.write(json.dumps({"method":"item/completed","params":{"threadId":thread_id,"turnId":turn_id,"itemId":"item_effort_probe","item":{"type":"agent_message","text":str(effort)}}}) + "\n")
        sys.stdout.write(json.dumps({"method":"turn/completed","params":{"threadId":thread_id,"turnId":turn_id}}) + "\n")
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"turn": {"id": turn_id}}}) + "\n")
        sys.stdout.flush()
        continue

    sys.stdout.write(json.dumps({"id": rpc_id, "result": {"echoMethod": method, "params": params}}) + "\n")
    sys.stdout.flush()
"#;

    let mut spec = StdioProcessSpec::new("python3");
    spec.args = vec!["-u".to_owned(), "-c".to_owned(), script.to_owned()];
    spec
}

fn python_run_prompt_mutation_probe_process() -> StdioProcessSpec {
    let script = r#"
import json
import sys

thread_model = {}

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

    if method == "initialize" and rpc_id is not None:
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"ready": True}}) + "\n")
        sys.stdout.flush()
        continue

    if rpc_id is None:
        continue

    if method == "thread/start":
        thread_id = "thr_mutation_probe"
        thread_model[thread_id] = params.get("model")
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"thread": {"id": thread_id}}}) + "\n")
        sys.stdout.write(json.dumps({"method":"thread/started","params":{"threadId":thread_id}}) + "\n")
        sys.stdout.flush()
        continue

    if method == "turn/start":
        thread_id = params.get("threadId", "thr_mutation_probe")
        turn_id = "turn_mutation_probe"
        input_items = params.get("input") or []
        text_value = ""
        item_types = []
        for item in input_items:
            t = item.get("type")
            if t is not None:
                item_types.append(t)
            if t == "text" and text_value == "":
                text_value = item.get("text", "")
        payload = {
            "threadModel": thread_model.get(thread_id),
            "turnModel": params.get("model"),
            "text": text_value,
            "itemTypes": item_types,
        }
        message = json.dumps(payload, sort_keys=True)
        sys.stdout.write(json.dumps({"method":"turn/started","params":{"threadId":thread_id,"turnId":turn_id}}) + "\n")
        sys.stdout.write(json.dumps({"method":"item/started","params":{"threadId":thread_id,"turnId":turn_id,"itemId":"item_mutation_probe","itemType":"agentMessage"}}) + "\n")
        sys.stdout.write(json.dumps({"method":"item/agentMessage/delta","params":{"threadId":thread_id,"turnId":turn_id,"itemId":"item_mutation_probe","delta":message}}) + "\n")
        sys.stdout.write(json.dumps({"method":"item/completed","params":{"threadId":thread_id,"turnId":turn_id,"itemId":"item_mutation_probe","item":{"type":"agent_message","text":message}}}) + "\n")
        sys.stdout.write(json.dumps({"method":"turn/completed","params":{"threadId":thread_id,"turnId":turn_id}}) + "\n")
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"turn": {"id": turn_id}}}) + "\n")
        sys.stdout.flush()
        continue

    sys.stdout.write(json.dumps({"id": rpc_id, "result": {"echoMethod": method, "params": params}}) + "\n")
    sys.stdout.flush()
"#;

    let mut spec = StdioProcessSpec::new("python3");
    spec.args = vec!["-u".to_owned(), "-c".to_owned(), script.to_owned()];
    spec
}

fn python_session_mutation_probe_process() -> StdioProcessSpec {
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
    params = msg.get("params") or {}

    if method == "initialize" and rpc_id is not None:
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"ready": True}}) + "\n")
        sys.stdout.flush()
        continue

    if rpc_id is None:
        continue

    if method == "thread/start":
        model = params.get("model") or "none"
        thread_id = f"thr_{model}"
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"thread": {"id": thread_id}}}) + "\n")
        sys.stdout.flush()
        continue

    if method == "thread/resume":
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"threadId": params.get("threadId", "thr_resume")}}) + "\n")
        sys.stdout.flush()
        continue

    sys.stdout.write(json.dumps({"id": rpc_id, "result": {"echoMethod": method, "params": params}}) + "\n")
    sys.stdout.flush()
"#;

    let mut spec = StdioProcessSpec::new("python3");
    spec.args = vec!["-u".to_owned(), "-c".to_owned(), script.to_owned()];
    spec
}

fn python_run_prompt_streaming_timeout_process() -> StdioProcessSpec {
    let script = r#"
import json
import sys
import time

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

    if method == "initialize" and rpc_id is not None:
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"ready": True}}) + "\n")
        sys.stdout.flush()
        continue

    if rpc_id is None:
        continue

    if method == "thread/start":
        thread_id = "thr_stream_timeout"
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"thread": {"id": thread_id}}}) + "\n")
        sys.stdout.write(json.dumps({"method":"thread/started","params":{"threadId":thread_id}}) + "\n")
        sys.stdout.flush()
        continue

    if method == "turn/start":
        thread_id = params.get("threadId", "thr_stream_timeout")
        turn_id = "turn_stream_timeout"
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"turn": {"id": turn_id}}}) + "\n")
        sys.stdout.write(json.dumps({"method":"turn/started","params":{"threadId":thread_id,"turnId":turn_id}}) + "\n")
        sys.stdout.flush()

        for _ in range(8):
            sys.stdout.write(json.dumps({"method":"item/agentMessage/delta","params":{"threadId":thread_id,"turnId":turn_id,"itemId":"item_stream_timeout","delta":"x"}}) + "\n")
            sys.stdout.flush()
            time.sleep(0.04)
        continue

    sys.stdout.write(json.dumps({"id": rpc_id, "result": {"echoMethod": method, "params": params}}) + "\n")
    sys.stdout.flush()
"#;

    let mut spec = StdioProcessSpec::new("python3");
    spec.args = vec!["-u".to_owned(), "-c".to_owned(), script.to_owned()];
    spec
}

fn python_run_prompt_interrupt_probe_process() -> StdioProcessSpec {
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
    params = msg.get("params") or {}

    if method == "initialize" and rpc_id is not None:
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"ready": True}}) + "\n")
        sys.stdout.flush()
        continue

    if method == "turn/interrupt":
        if rpc_id is None:
            # Interrupt must be an RPC request; ignore notifications.
            continue
        sys.stdout.write(json.dumps({"method":"probe/interruptSeen","params":{"threadId":params.get("threadId"),"turnId":params.get("turnId")}}) + "\n")
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"ok": True}}) + "\n")
        sys.stdout.flush()
        continue

    if rpc_id is None:
        continue

    if method == "thread/start":
        thread_id = "thr_interrupt_probe"
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"thread": {"id": thread_id}}}) + "\n")
        sys.stdout.write(json.dumps({"method":"thread/started","params":{"threadId":thread_id}}) + "\n")
        sys.stdout.flush()
        continue

    if method == "turn/start":
        thread_id = params.get("threadId", "thr_interrupt_probe")
        turn_id = "turn_interrupt_probe"
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"turn": {"id": turn_id}}}) + "\n")
        sys.stdout.write(json.dumps({"method":"turn/started","params":{"threadId":thread_id,"turnId":turn_id}}) + "\n")
        sys.stdout.flush()
        continue

    sys.stdout.write(json.dumps({"id": rpc_id, "result": {"echoMethod": method, "params": params}}) + "\n")
    sys.stdout.flush()
"#;

    let mut spec = StdioProcessSpec::new("python3");
    spec.args = vec!["-u".to_owned(), "-c".to_owned(), script.to_owned()];
    spec
}

fn python_thread_resume_missing_id_process() -> StdioProcessSpec {
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

    if rpc_id is None:
        continue

    if method == "thread/resume":
        # Deliberately omit thread id to validate client-side contract checks.
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

fn python_thread_resume_mismatched_id_process() -> StdioProcessSpec {
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

    if rpc_id is None:
        continue

    if method == "thread/resume":
        # Deliberately return an id different from the requested one.
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"threadId": "thr_unexpected"}}) + "\n")
        sys.stdout.flush()
        continue

    sys.stdout.write(json.dumps({"id": rpc_id, "result": {"echoMethod": method}}) + "\n")
    sys.stdout.flush()
"#;

    let mut spec = StdioProcessSpec::new("python3");
    spec.args = vec!["-u".to_owned(), "-c".to_owned(), script.to_owned()];
    spec
}

fn python_run_prompt_lagged_completion_process() -> StdioProcessSpec {
    let script = r#"
import json
import sys

def make_thread(thread_id):
    return {
        "id": thread_id,
        "cliVersion": "0.104.0",
        "createdAt": 1700000000,
        "cwd": "/tmp",
        "modelProvider": "openai",
        "path": f"/tmp/threads/{thread_id}.jsonl",
        "preview": "hello",
        "source": "app-server",
        "turns": [],
        "updatedAt": 1700000001,
    }

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

    if method == "initialize" and rpc_id is not None:
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"ready": True}}) + "\n")
        sys.stdout.flush()
        continue

    if rpc_id is None:
        continue

    if method == "thread/start":
        thread_id = "thr_lagged"
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"thread": {"id": thread_id}}}) + "\n")
        sys.stdout.write(json.dumps({"method":"thread/started","params":{"threadId":thread_id}}) + "\n")
        sys.stdout.flush()
        continue

    if method == "turn/start":
        thread_id = params.get("threadId", "thr_lagged")
        turn_id = "turn_lagged"
        sys.stdout.write(json.dumps({"method":"turn/started","params":{"threadId":thread_id,"turnId":turn_id}}) + "\n")
        sys.stdout.write(json.dumps({"method":"item/started","params":{"threadId":thread_id,"turnId":turn_id,"itemId":"item_lagged","itemType":"agentMessage"}}) + "\n")
        for i in range(8):
            sys.stdout.write(json.dumps({"method":"item/agentMessage/delta","params":{"threadId":thread_id,"turnId":turn_id,"itemId":"item_lagged","delta":f"chunk-{i}"}}) + "\n")
        # Terminal event may be dropped when live receiver lags.
        sys.stdout.write(json.dumps({"method":"turn/completed","params":{"threadId":thread_id,"turnId":turn_id}}) + "\n")
        # Keep one non-terminal tail event so a lagged receiver cannot rely on stream terminal events.
        sys.stdout.write(json.dumps({"method":"item/started","params":{"threadId":thread_id,"turnId":turn_id,"itemId":"item_tail","itemType":"reasoning"}}) + "\n")
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"turn": {"id": turn_id}}}) + "\n")
        sys.stdout.flush()
        continue

    if method == "thread/read":
        thread_id = params.get("threadId", "thr_lagged")
        thread = make_thread(thread_id)
        if params.get("includeTurns"):
            thread["turns"] = [{
                "id": "turn_lagged",
                "status": "completed",
                "items": [
                    {"id": "item_lagged_final", "type": "agentMessage", "text": "ok-from-thread-read"}
                ],
            }]
        out = {"id": rpc_id, "result": {"thread": thread}}
        sys.stdout.write(json.dumps(out) + "\n")
        sys.stdout.flush()
        continue

    sys.stdout.write(json.dumps({"id": rpc_id, "result": {"echoMethod": method, "params": params}}) + "\n")
    sys.stdout.flush()
"#;

    let mut spec = StdioProcessSpec::new("python3");
    spec.args = vec!["-u".to_owned(), "-c".to_owned(), script.to_owned()];
    spec
}

async fn spawn_mock_runtime() -> Runtime {
    let cfg = RuntimeConfig::new(python_api_mock_process(), workspace_schema_guard());
    Runtime::spawn_local(cfg).await.expect("spawn runtime")
}

async fn spawn_run_prompt_runtime() -> Runtime {
    let cfg = RuntimeConfig::new(python_run_prompt_mock_process(), workspace_schema_guard());
    Runtime::spawn_local(cfg).await.expect("spawn runtime")
}

async fn spawn_run_prompt_cross_thread_noise_runtime() -> Runtime {
    let cfg = RuntimeConfig::new(
        python_run_prompt_cross_thread_noise_process(),
        workspace_schema_guard(),
    );
    Runtime::spawn_local(cfg).await.expect("spawn runtime")
}

async fn spawn_run_prompt_error_runtime() -> Runtime {
    let cfg = RuntimeConfig::new(
        python_run_prompt_error_mock_process(),
        workspace_schema_guard(),
    );
    Runtime::spawn_local(cfg).await.expect("spawn runtime")
}

async fn spawn_run_prompt_turn_failed_runtime() -> Runtime {
    let cfg = RuntimeConfig::new(
        python_run_prompt_turn_failed_mock_process(),
        workspace_schema_guard(),
    );
    Runtime::spawn_local(cfg).await.expect("spawn runtime")
}

async fn spawn_run_prompt_effort_probe_runtime() -> Runtime {
    let cfg = RuntimeConfig::new(
        python_run_prompt_effort_probe_process(),
        workspace_schema_guard(),
    );
    Runtime::spawn_local(cfg).await.expect("spawn runtime")
}

async fn spawn_run_prompt_mutation_probe_runtime(hooks: RuntimeHookConfig) -> Runtime {
    let cfg = RuntimeConfig::new(
        python_run_prompt_mutation_probe_process(),
        workspace_schema_guard(),
    )
    .with_hooks(hooks);
    Runtime::spawn_local(cfg).await.expect("spawn runtime")
}

async fn spawn_run_prompt_streaming_timeout_runtime() -> Runtime {
    let cfg = RuntimeConfig::new(
        python_run_prompt_streaming_timeout_process(),
        workspace_schema_guard(),
    );
    Runtime::spawn_local(cfg).await.expect("spawn runtime")
}

async fn spawn_run_prompt_interrupt_probe_runtime() -> Runtime {
    let cfg = RuntimeConfig::new(
        python_run_prompt_interrupt_probe_process(),
        workspace_schema_guard(),
    );
    Runtime::spawn_local(cfg).await.expect("spawn runtime")
}

async fn spawn_thread_resume_missing_id_runtime() -> Runtime {
    let cfg = RuntimeConfig::new(
        python_thread_resume_missing_id_process(),
        workspace_schema_guard(),
    );
    Runtime::spawn_local(cfg).await.expect("spawn runtime")
}

async fn spawn_thread_resume_mismatched_id_runtime() -> Runtime {
    let cfg = RuntimeConfig::new(
        python_thread_resume_mismatched_id_process(),
        workspace_schema_guard(),
    );
    Runtime::spawn_local(cfg).await.expect("spawn runtime")
}

async fn spawn_run_prompt_lagged_completion_runtime() -> Runtime {
    let mut cfg = RuntimeConfig::new(
        python_run_prompt_lagged_completion_process(),
        workspace_schema_guard(),
    );
    cfg.live_channel_capacity = 1;
    Runtime::spawn_local(cfg).await.expect("spawn runtime")
}

#[derive(Clone)]
struct RecordingPreHook {
    name: &'static str,
    events: Arc<Mutex<Vec<String>>>,
    fail_phase: Option<HookPhase>,
}

impl PreHook for RecordingPreHook {
    fn name(&self) -> &'static str {
        self.name
    }

    fn call<'a>(
        &'a self,
        ctx: &'a HookContext,
    ) -> coclai_plugin_core::HookFuture<'a, Result<HookAction, HookIssue>> {
        Box::pin(async move {
            self.events
                .lock()
                .expect("events lock")
                .push(format!("pre:{:?}", ctx.phase));
            if self.fail_phase == Some(ctx.phase) {
                return Err(HookIssue {
                    hook_name: self.name.to_owned(),
                    phase: ctx.phase,
                    class: HookIssueClass::Execution,
                    message: "forced pre hook failure".to_owned(),
                });
            }
            Ok(HookAction::Noop)
        })
    }
}

#[derive(Clone)]
struct RecordingPostHook {
    name: &'static str,
    events: Arc<Mutex<Vec<String>>>,
    fail_phase: Option<HookPhase>,
}

impl PostHook for RecordingPostHook {
    fn name(&self) -> &'static str {
        self.name
    }

    fn call<'a>(
        &'a self,
        ctx: &'a HookContext,
    ) -> coclai_plugin_core::HookFuture<'a, Result<(), HookIssue>> {
        Box::pin(async move {
            self.events
                .lock()
                .expect("events lock")
                .push(format!("post:{:?}", ctx.phase));
            if self.fail_phase == Some(ctx.phase) {
                return Err(HookIssue {
                    hook_name: self.name.to_owned(),
                    phase: ctx.phase,
                    class: HookIssueClass::Execution,
                    message: "forced post hook failure".to_owned(),
                });
            }
            Ok(())
        })
    }
}

#[derive(Clone)]
struct PhasePatchPreHook {
    name: &'static str,
    patches: Vec<(HookPhase, coclai_plugin_core::HookPatch)>,
}

impl PreHook for PhasePatchPreHook {
    fn name(&self) -> &'static str {
        self.name
    }

    fn call<'a>(
        &'a self,
        ctx: &'a HookContext,
    ) -> coclai_plugin_core::HookFuture<'a, Result<HookAction, HookIssue>> {
        Box::pin(async move {
            if let Some((_, patch)) = self.patches.iter().find(|(phase, _)| *phase == ctx.phase) {
                Ok(HookAction::Mutate(patch.clone()))
            } else {
                Ok(HookAction::Noop)
            }
        })
    }
}

#[derive(Clone)]
struct MetadataCapturePostHook {
    name: &'static str,
    metadata: Arc<Mutex<Vec<(HookPhase, Value)>>>,
}

impl PostHook for MetadataCapturePostHook {
    fn name(&self) -> &'static str {
        self.name
    }

    fn call<'a>(
        &'a self,
        ctx: &'a HookContext,
    ) -> coclai_plugin_core::HookFuture<'a, Result<(), HookIssue>> {
        Box::pin(async move {
            self.metadata
                .lock()
                .expect("metadata lock")
                .push((ctx.phase, ctx.metadata.clone()));
            Ok(())
        })
    }
}

async fn spawn_run_prompt_runtime_with_hooks(hooks: RuntimeHookConfig) -> Runtime {
    let cfg = RuntimeConfig::new(python_run_prompt_mock_process(), workspace_schema_guard())
        .with_hooks(hooks);
    Runtime::spawn_local(cfg).await.expect("spawn runtime")
}

#[path = "tests/params_and_types.rs"]
mod params_and_types;
#[path = "tests/run_prompt.rs"]
mod run_prompt;
#[path = "tests/thread_api.rs"]
mod thread_api;
