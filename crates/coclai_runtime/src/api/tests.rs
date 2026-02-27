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

#[test]
fn maps_turn_start_params_to_wire_shape() {
    let params = TurnStartParams {
        input: vec![
            InputItem::Text {
                text: "hello".to_owned(),
            },
            InputItem::LocalImage {
                path: "/tmp/a.png".to_owned(),
            },
        ],
        cwd: Some("/tmp".to_owned()),
        approval_policy: Some(ApprovalPolicy::Never),
        sandbox_policy: Some(SandboxPolicy::Preset(SandboxPreset::WorkspaceWrite {
            writable_roots: vec!["/tmp".to_owned()],
            network_access: false,
        })),
        privileged_escalation_approved: true,
        model: Some("gpt-5".to_owned()),
        effort: Some(ReasoningEffort::High),
        summary: Some("brief".to_owned()),
        output_schema: Some(json!({"type":"object"})),
    };

    let wire = turn_start_params_to_wire("thr_1", &params);
    assert_eq!(wire["threadId"], "thr_1");
    assert_eq!(wire["input"][0]["type"], "text");
    assert_eq!(wire["input"][0]["text"], "hello");
    assert_eq!(wire["input"][1]["type"], "localImage");
    assert_eq!(wire["input"][1]["path"], "/tmp/a.png");
    assert_eq!(wire["approvalPolicy"], "never");
    assert_eq!(wire["sandboxPolicy"]["type"], "workspaceWrite");
    assert_eq!(wire["sandboxPolicy"]["writableRoots"][0], "/tmp");
    assert_eq!(wire["sandboxPolicy"]["networkAccess"], false);
    assert_eq!(wire["outputSchema"]["type"], "object");
}

#[test]
fn maps_text_with_elements_input_to_wire_shape() {
    let input = InputItem::TextWithElements {
        text: "check @README.md".to_owned(),
        text_elements: vec![TextElement {
            byte_range: ByteRange { start: 6, end: 16 },
            placeholder: Some("README".to_owned()),
        }],
    };
    let wire = input_item_to_wire(&input);
    assert_eq!(wire["type"], "text");
    assert_eq!(wire["text"], "check @README.md");
    assert_eq!(wire["text_elements"][0]["byteRange"]["start"], 6);
    assert_eq!(wire["text_elements"][0]["byteRange"]["end"], 16);
    assert_eq!(wire["text_elements"][0]["placeholder"], "README");
}

#[test]
fn builds_prompt_input_with_at_path_attachment() {
    let input = build_prompt_inputs(
        "summarize",
        &[PromptAttachment::AtPath {
            path: "README.md".to_owned(),
            placeholder: None,
        }],
    );
    assert_eq!(input.len(), 1);
    match &input[0] {
        InputItem::TextWithElements {
            text,
            text_elements,
        } => {
            assert_eq!(text, "summarize\n@README.md");
            assert_eq!(text_elements.len(), 1);
            assert_eq!(text_elements[0].byte_range.start, 10);
            assert_eq!(text_elements[0].byte_range.end, 20);
        }
        other => panic!("unexpected input variant: {other:?}"),
    }
}

#[test]
fn parses_policy_and_effort_from_str() {
    assert_eq!(
        ApprovalPolicy::from_str("on-request").expect("parse approval"),
        ApprovalPolicy::OnRequest
    );
    assert_eq!(
        ReasoningEffort::from_str("xhigh").expect("parse effort"),
        ReasoningEffort::XHigh
    );
    assert_eq!(
        ThreadListSortKey::from_str("updated_at").expect("parse thread list sort key"),
        ThreadListSortKey::UpdatedAt
    );
    assert!(ApprovalPolicy::from_str("always").is_err());
    assert!(ReasoningEffort::from_str("ultra").is_err());
    assert!(ThreadListSortKey::from_str("latest").is_err());

    let known_item_type: ThreadItemType =
        serde_json::from_value(json!("agentMessage")).expect("parse known item type");
    assert_eq!(known_item_type, ThreadItemType::AgentMessage);

    let unknown_item_type: ThreadItemType =
        serde_json::from_value(json!("futureType")).expect("parse unknown item type");
    assert_eq!(
        unknown_item_type,
        ThreadItemType::Unknown("futureType".to_owned())
    );
    assert_eq!(
        serde_json::to_value(&unknown_item_type).expect("serialize unknown item type"),
        json!("futureType")
    );
}

#[test]
fn parses_thread_item_payload_variants() {
    let agent: ThreadItemView = serde_json::from_value(json!({
        "id": "item_a",
        "type": "agentMessage",
        "text": "hello"
    }))
    .expect("parse agent item");
    assert_eq!(agent.id, "item_a");
    assert_eq!(agent.item_type, ThreadItemType::AgentMessage);
    match agent.payload {
        ThreadItemPayloadView::AgentMessage(data) => assert_eq!(data.text, "hello"),
        other => panic!("unexpected payload: {other:?}"),
    }

    let command: ThreadItemView = serde_json::from_value(json!({
        "id": "item_c",
        "type": "commandExecution",
        "command": "echo hi",
        "commandActions": [],
        "cwd": "/tmp",
        "status": "completed"
    }))
    .expect("parse command item");
    match command.payload {
        ThreadItemPayloadView::CommandExecution(data) => {
            assert_eq!(data.command, "echo hi");
            assert_eq!(data.cwd, "/tmp");
            assert_eq!(data.status, "completed");
        }
        other => panic!("unexpected payload: {other:?}"),
    }

    let unknown: ThreadItemView = serde_json::from_value(json!({
        "id": "item_u",
        "type": "futureType",
        "foo": "bar"
    }))
    .expect("parse unknown item");
    assert_eq!(
        unknown.item_type,
        ThreadItemType::Unknown("futureType".to_owned())
    );
    match unknown.payload {
        ThreadItemPayloadView::Unknown(fields) => {
            assert_eq!(fields.get("foo"), Some(&json!("bar")));
        }
        other => panic!("unexpected payload: {other:?}"),
    }
}

#[test]
fn validate_prompt_attachments_rejects_missing_path() {
    let err = validate_prompt_attachments(
        "/tmp",
        &[PromptAttachment::AtPath {
            path: "definitely_missing_file_12345.txt".to_owned(),
            placeholder: None,
        }],
    )
    .expect_err("must fail");
    match err {
        PromptRunError::AttachmentNotFound(path) => {
            assert!(path.ends_with("/tmp/definitely_missing_file_12345.txt"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn prompt_run_params_defaults_are_explicit() {
    let params = PromptRunParams::new("/work", "hello");
    assert_eq!(params.cwd, "/work");
    assert_eq!(params.prompt, "hello");
    assert_eq!(params.effort, Some(DEFAULT_REASONING_EFFORT));
    assert_eq!(params.approval_policy, ApprovalPolicy::Never);
    assert_eq!(
        params.sandbox_policy,
        SandboxPolicy::Preset(SandboxPreset::ReadOnly)
    );
    assert!(!params.privileged_escalation_approved);
    assert_eq!(params.timeout, Duration::from_secs(120));
    assert!(params.attachments.is_empty());
}

#[test]
fn prompt_run_params_builder_overrides_defaults() {
    let params = PromptRunParams::new("/work", "hello")
        .with_model("gpt-5-codex")
        .with_effort(ReasoningEffort::High)
        .with_approval_policy(ApprovalPolicy::OnRequest)
        .with_sandbox_policy(SandboxPolicy::Preset(SandboxPreset::WorkspaceWrite {
            writable_roots: vec!["/work".to_owned()],
            network_access: false,
        }))
        .allow_privileged_escalation()
        .attach_path("README.md")
        .attach_path_with_placeholder("Docs/CORE_API.md", "core-doc")
        .attach_image_url("https://example.com/a.png")
        .attach_local_image("/tmp/a.png")
        .attach_skill("checks", "/tmp/skill")
        .with_timeout(Duration::from_secs(30));

    assert_eq!(params.cwd, "/work");
    assert_eq!(params.prompt, "hello");
    assert_eq!(params.model.as_deref(), Some("gpt-5-codex"));
    assert_eq!(params.effort, Some(ReasoningEffort::High));
    assert_eq!(params.approval_policy, ApprovalPolicy::OnRequest);
    assert_eq!(
        params.sandbox_policy,
        SandboxPolicy::Preset(SandboxPreset::WorkspaceWrite {
            writable_roots: vec!["/work".to_owned()],
            network_access: false,
        })
    );
    assert!(params.privileged_escalation_approved);
    assert_eq!(params.timeout, Duration::from_secs(30));
    assert_eq!(params.attachments.len(), 5);
    assert!(matches!(
        params.attachments[0],
        PromptAttachment::AtPath {
            ref path,
            placeholder: None
        } if path == "README.md"
    ));
    assert!(matches!(
        params.attachments[1],
        PromptAttachment::AtPath {
            ref path,
            placeholder: Some(ref placeholder)
        } if path == "Docs/CORE_API.md" && placeholder == "core-doc"
    ));
    assert!(matches!(
        params.attachments[2],
        PromptAttachment::ImageUrl { ref url } if url == "https://example.com/a.png"
    ));
    assert!(matches!(
        params.attachments[3],
        PromptAttachment::LocalImage { ref path } if path == "/tmp/a.png"
    ));
    assert!(matches!(
        params.attachments[4],
        PromptAttachment::Skill {
            ref name,
            ref path
        } if name == "checks" && path == "/tmp/skill"
    ));
}

#[test]
fn maps_thread_start_params_to_wire_shape() {
    let params = ThreadStartParams {
        model: Some("gpt-5".to_owned()),
        cwd: Some("/work".to_owned()),
        approval_policy: Some(ApprovalPolicy::OnRequest),
        sandbox_policy: Some(SandboxPolicy::Preset(SandboxPreset::ReadOnly)),
        privileged_escalation_approved: false,
    };

    let wire = thread_start_params_to_wire(&params);
    assert_eq!(wire["model"], "gpt-5");
    assert_eq!(wire["cwd"], "/work");
    assert_eq!(wire["approvalPolicy"], "on-request");
    assert_eq!(wire["sandbox"], "read-only");
}

#[tokio::test(flavor = "current_thread")]
async fn typed_thread_and_turn_roundtrip() {
    let runtime = spawn_mock_runtime().await;

    let thread = runtime
        .thread_start(ThreadStartParams {
            approval_policy: Some(ApprovalPolicy::Never),
            sandbox_policy: Some(SandboxPolicy::Preset(SandboxPreset::ReadOnly)),
            ..ThreadStartParams::default()
        })
        .await
        .expect("thread start");
    assert_eq!(thread.thread_id, "thr_typed");

    let turn = thread
        .turn_start(TurnStartParams {
            input: vec![InputItem::Text {
                text: "hi".to_owned(),
            }],
            ..TurnStartParams::default()
        })
        .await
        .expect("turn start");
    assert_eq!(turn.thread_id, "thr_typed");
    assert_eq!(turn.turn_id, "turn_typed");

    let steered = thread
        .turn_steer(
            &turn.turn_id,
            vec![InputItem::Text {
                text: "continue".to_owned(),
            }],
        )
        .await
        .expect("turn steer");
    assert_eq!(steered, "turn_typed");

    thread
        .turn_interrupt(&turn.turn_id)
        .await
        .expect("turn interrupt");

    let resumed = runtime
        .thread_resume("thr_old", ThreadStartParams::default())
        .await
        .expect("thread resume");
    assert_eq!(resumed.thread_id, "thr_old");

    let forked = runtime.thread_fork("thr_old").await.expect("thread fork");
    assert_eq!(forked.thread_id, "thr_forked");

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn thread_start_rejects_privileged_sandbox_without_explicit_opt_in() {
    let runtime = spawn_mock_runtime().await;

    let err = runtime
        .thread_start(ThreadStartParams {
            cwd: Some("/tmp".to_owned()),
            approval_policy: Some(ApprovalPolicy::OnRequest),
            sandbox_policy: Some(SandboxPolicy::Preset(SandboxPreset::DangerFullAccess)),
            privileged_escalation_approved: false,
            ..ThreadStartParams::default()
        })
        .await
        .expect_err("must reject privileged sandbox without explicit opt-in");
    assert!(matches!(err, crate::errors::RpcError::InvalidRequest(_)));

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn thread_start_rejects_privileged_sandbox_without_non_never_approval() {
    let runtime = spawn_mock_runtime().await;

    let err = runtime
        .thread_start(ThreadStartParams {
            cwd: Some("/tmp".to_owned()),
            approval_policy: Some(ApprovalPolicy::Never),
            sandbox_policy: Some(SandboxPolicy::Preset(SandboxPreset::DangerFullAccess)),
            privileged_escalation_approved: true,
            ..ThreadStartParams::default()
        })
        .await
        .expect_err("must reject privileged sandbox with never approval");
    assert!(matches!(err, crate::errors::RpcError::InvalidRequest(_)));

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn thread_start_rejects_privileged_sandbox_without_scope() {
    let runtime = spawn_mock_runtime().await;

    let err = runtime
        .thread_start(ThreadStartParams {
            cwd: None,
            approval_policy: Some(ApprovalPolicy::OnRequest),
            sandbox_policy: Some(SandboxPolicy::Preset(SandboxPreset::DangerFullAccess)),
            privileged_escalation_approved: true,
            ..ThreadStartParams::default()
        })
        .await
        .expect_err("must reject privileged sandbox without explicit scope");
    assert!(matches!(err, crate::errors::RpcError::InvalidRequest(_)));

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn turn_start_rejects_privileged_sandbox_without_explicit_opt_in() {
    let runtime = spawn_mock_runtime().await;
    let thread = runtime
        .thread_start(ThreadStartParams {
            approval_policy: Some(ApprovalPolicy::Never),
            sandbox_policy: Some(SandboxPolicy::Preset(SandboxPreset::ReadOnly)),
            ..ThreadStartParams::default()
        })
        .await
        .expect("thread start");

    let err = thread
        .turn_start(TurnStartParams {
            input: vec![InputItem::Text {
                text: "hi".to_owned(),
            }],
            cwd: Some("/tmp".to_owned()),
            approval_policy: Some(ApprovalPolicy::OnRequest),
            sandbox_policy: Some(SandboxPolicy::Preset(SandboxPreset::WorkspaceWrite {
                writable_roots: vec!["/tmp".to_owned()],
                network_access: false,
            })),
            privileged_escalation_approved: false,
            ..TurnStartParams::default()
        })
        .await
        .expect_err("must reject privileged turn without explicit opt-in");
    assert!(matches!(err, crate::errors::RpcError::InvalidRequest(_)));

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn thread_resume_requires_thread_id_in_response() {
    let runtime = spawn_thread_resume_missing_id_runtime().await;

    let err = runtime
        .thread_resume("thr_missing", ThreadStartParams::default())
        .await
        .expect_err("thread resume must fail without thread id in response");

    match err {
        RpcError::InvalidRequest(message) => {
            assert!(message.contains("thread/resume missing thread id in result"));
        }
        other => panic!("unexpected error: {other:?}"),
    }

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn thread_resume_requires_matching_thread_id() {
    let runtime = spawn_thread_resume_mismatched_id_runtime().await;

    let err = runtime
        .thread_resume("thr_expected", ThreadStartParams::default())
        .await
        .expect_err("thread resume must fail on mismatched thread id in response");

    match err {
        RpcError::InvalidRequest(message) => {
            assert!(message.contains("thread/resume returned mismatched thread id"));
            assert!(message.contains("requested=thr_expected"));
            assert!(message.contains("actual=thr_unexpected"));
        }
        other => panic!("unexpected error: {other:?}"),
    }

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn turn_start_rejects_empty_input() {
    let runtime = spawn_mock_runtime().await;
    let thread = runtime
        .thread_start(ThreadStartParams::default())
        .await
        .expect("thread start");

    let err = thread
        .turn_start(TurnStartParams::default())
        .await
        .expect_err("must fail");
    assert!(matches!(err, RpcError::InvalidRequest(_)));

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn runtime_archive_and_interrupt_wrappers_work() {
    let runtime = spawn_mock_runtime().await;

    runtime
        .turn_interrupt("thr_typed", "turn_typed")
        .await
        .expect("runtime turn interrupt");
    runtime
        .thread_archive("thr_typed")
        .await
        .expect("runtime thread archive");

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn runtime_thread_read_list_loaded_and_rollback_wrappers_work() {
    let runtime = spawn_mock_runtime().await;

    let read = runtime
        .thread_read(ThreadReadParams {
            thread_id: "thr_typed".to_owned(),
            include_turns: Some(true),
        })
        .await
        .expect("thread read");
    assert_eq!(read.thread.id, "thr_typed");
    assert_eq!(read.thread.source, "app-server");
    assert_eq!(read.thread.extra.get("turnsIncluded"), Some(&json!(true)));
    assert_eq!(read.thread.turns.len(), 1);
    assert_eq!(read.thread.turns[0].id, "turn_read_1");
    assert_eq!(read.thread.turns[0].status, ThreadTurnStatus::Completed);
    assert_eq!(read.thread.turns[0].items.len(), 1);
    assert_eq!(
        read.thread.turns[0].items[0].item_type,
        ThreadItemType::AgentMessage
    );
    match &read.thread.turns[0].items[0].payload {
        ThreadItemPayloadView::AgentMessage(data) => assert_eq!(data.text, "ok"),
        other => panic!("unexpected payload: {other:?}"),
    }

    let listed = runtime
        .thread_list(ThreadListParams {
            archived: Some(true),
            cursor: Some("cursor_a".to_owned()),
            limit: Some(5),
            model_providers: Some(vec!["openai".to_owned(), "anthropic".to_owned()]),
            sort_key: Some(ThreadListSortKey::UpdatedAt),
        })
        .await
        .expect("thread list");
    assert_eq!(listed.data.len(), 1);
    assert_eq!(listed.data[0].id, "thr_list");
    assert_eq!(listed.data[0].model_provider, "openai");
    assert_eq!(
        listed.data[0].extra.get("archivedFilter"),
        Some(&json!(true))
    );
    assert_eq!(
        listed.data[0].extra.get("sortKey"),
        Some(&json!("updated_at"))
    );
    assert_eq!(listed.data[0].extra.get("providerCount"), Some(&json!(2)));
    assert_eq!(listed.next_cursor.as_deref(), Some("cursor_a"));

    let loaded = runtime
        .thread_loaded_list(ThreadLoadedListParams {
            cursor: Some("loaded_cursor".to_owned()),
            limit: Some(1),
        })
        .await
        .expect("thread loaded list");
    assert_eq!(loaded.data, vec!["thr_loaded_1".to_owned()]);
    assert_eq!(loaded.next_cursor.as_deref(), Some("loaded_cursor"));

    let rollback = runtime
        .thread_rollback(ThreadRollbackParams {
            thread_id: "thr_typed".to_owned(),
            num_turns: 3,
        })
        .await
        .expect("thread rollback");
    assert_eq!(rollback.thread.id, "thr_typed");
    assert_eq!(
        rollback.thread.extra.get("rolledBackTurns"),
        Some(&json!(3))
    );
    assert_eq!(rollback.thread.turns.len(), 1);
    assert_eq!(rollback.thread.turns[0].status, ThreadTurnStatus::Failed);
    assert_eq!(
        rollback.thread.turns[0].items[0].item_type,
        ThreadItemType::CommandExecution
    );
    match &rollback.thread.turns[0].items[0].payload {
        ThreadItemPayloadView::CommandExecution(data) => {
            assert_eq!(data.command, "false");
            assert_eq!(data.status, "failed");
        }
        other => panic!("unexpected payload: {other:?}"),
    }

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn run_prompt_returns_assistant_text() {
    let runtime = spawn_run_prompt_runtime().await;
    let result = runtime
        .run_prompt(PromptRunParams {
            cwd: "/tmp".to_owned(),
            prompt: "say ok".to_owned(),
            model: Some("gpt-5-codex".to_owned()),
            effort: Some(ReasoningEffort::High),
            approval_policy: ApprovalPolicy::Never,
            sandbox_policy: SandboxPolicy::Preset(SandboxPreset::ReadOnly),
            privileged_escalation_approved: false,
            attachments: vec![],
            timeout: Duration::from_secs(2),
        })
        .await
        .expect("run prompt");

    assert_eq!(result.thread_id, "thr_prompt");
    assert_eq!(result.turn_id, "turn_prompt");
    assert_eq!(result.assistant_text, "ok-from-run-prompt");

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn run_prompt_simple_returns_assistant_text() {
    let runtime = spawn_run_prompt_runtime().await;
    let result = runtime
        .run_prompt_simple("/tmp", "say ok")
        .await
        .expect("run prompt simple");

    assert_eq!(result.thread_id, "thr_prompt");
    assert_eq!(result.turn_id, "turn_prompt");
    assert_eq!(result.assistant_text, "ok-from-run-prompt");

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn run_prompt_in_thread_reuses_existing_thread_id() {
    let runtime = spawn_run_prompt_runtime().await;
    let result = runtime
        .run_prompt_in_thread(
            "thr_existing",
            PromptRunParams::new("/tmp", "continue conversation"),
        )
        .await
        .expect("run prompt in thread");

    assert_eq!(result.thread_id, "thr_existing");
    assert_eq!(result.turn_id, "turn_prompt");
    assert_eq!(result.assistant_text, "ok-from-run-prompt");

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn run_prompt_hook_order_is_pre_then_post() {
    let events = Arc::new(Mutex::new(Vec::<String>::new()));
    let hooks = RuntimeHookConfig::new()
        .with_pre_hook(Arc::new(RecordingPreHook {
            name: "pre_recorder",
            events: events.clone(),
            fail_phase: None,
        }))
        .with_post_hook(Arc::new(RecordingPostHook {
            name: "post_recorder",
            events: events.clone(),
            fail_phase: None,
        }));
    let runtime = spawn_run_prompt_runtime_with_hooks(hooks).await;

    let result = runtime
        .run_prompt(PromptRunParams::new("/tmp", "say ok"))
        .await
        .expect("run prompt");

    assert_eq!(result.thread_id, "thr_prompt");
    assert_eq!(result.turn_id, "turn_prompt");
    assert_eq!(result.assistant_text, "ok-from-run-prompt");
    assert!(
        runtime.hook_report_snapshot().is_clean(),
        "no hook issue expected"
    );
    assert_eq!(
        events.lock().expect("events lock").as_slice(),
        &[
            "pre:PreRun".to_owned(),
            "pre:PreTurn".to_owned(),
            "post:PostTurn".to_owned(),
            "post:PostRun".to_owned(),
        ]
    );

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn run_prompt_hook_failure_is_fail_open_with_report() {
    let events = Arc::new(Mutex::new(Vec::<String>::new()));
    let hooks = RuntimeHookConfig::new()
        .with_pre_hook(Arc::new(RecordingPreHook {
            name: "pre_fail",
            events: events.clone(),
            fail_phase: Some(HookPhase::PreRun),
        }))
        .with_post_hook(Arc::new(RecordingPostHook {
            name: "post_fail",
            events: events.clone(),
            fail_phase: Some(HookPhase::PostRun),
        }));
    let runtime = spawn_run_prompt_runtime_with_hooks(hooks).await;

    let result = runtime
        .run_prompt(PromptRunParams::new("/tmp", "say ok"))
        .await
        .expect("run prompt must continue despite hook failures");

    assert_eq!(result.assistant_text, "ok-from-run-prompt");
    let report = runtime.hook_report_snapshot();
    assert_eq!(report.issues.len(), 2);
    assert_eq!(report.issues[0].hook_name, "pre_fail");
    assert_eq!(report.issues[0].phase, HookPhase::PreRun);
    assert_eq!(report.issues[1].hook_name, "post_fail");
    assert_eq!(report.issues[1].phase, HookPhase::PostRun);
    assert_eq!(
        events.lock().expect("events lock").as_slice(),
        &[
            "pre:PreRun".to_owned(),
            "pre:PreTurn".to_owned(),
            "post:PostTurn".to_owned(),
            "post:PostRun".to_owned(),
        ]
    );

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn thread_start_and_resume_emit_session_hook_phases() {
    let events = Arc::new(Mutex::new(Vec::<String>::new()));
    let hooks = RuntimeHookConfig::new()
        .with_pre_hook(Arc::new(RecordingPreHook {
            name: "pre_session",
            events: events.clone(),
            fail_phase: None,
        }))
        .with_post_hook(Arc::new(RecordingPostHook {
            name: "post_session",
            events: events.clone(),
            fail_phase: None,
        }));
    let cfg =
        RuntimeConfig::new(python_api_mock_process(), workspace_schema_guard()).with_hooks(hooks);
    let runtime = Runtime::spawn_local(cfg).await.expect("spawn runtime");

    let started = runtime
        .thread_start(ThreadStartParams::default())
        .await
        .expect("thread start");
    assert_eq!(started.thread_id, "thr_typed");

    let resumed = runtime
        .thread_resume("thr_existing", ThreadStartParams::default())
        .await
        .expect("thread resume");
    assert_eq!(resumed.thread_id, "thr_existing");

    assert!(
        runtime.hook_report_snapshot().is_clean(),
        "session hooks should not report issue"
    );
    assert_eq!(
        events.lock().expect("events lock").as_slice(),
        &[
            "pre:PreSessionStart".to_owned(),
            "post:PostSessionStart".to_owned(),
            "pre:PreSessionStart".to_owned(),
            "post:PostSessionStart".to_owned(),
        ]
    );

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn run_prompt_applies_pre_mutations_for_prompt_model_attachment_and_metadata() {
    let existing_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../README.md")
        .to_string_lossy()
        .to_string();
    let patches = vec![
        (
            HookPhase::PreRun,
            coclai_plugin_core::HookPatch {
                prompt_override: Some("patched-in-pre-run".to_owned()),
                model_override: Some("model-pre-run".to_owned()),
                add_attachments: vec![coclai_plugin_core::HookAttachment::ImageUrl {
                    url: "https://example.com/x.png".to_owned(),
                }],
                metadata_delta: json!({"from_pre_run": true}),
            },
        ),
        (
            HookPhase::PreTurn,
            coclai_plugin_core::HookPatch {
                prompt_override: Some("patched-in-pre-turn".to_owned()),
                model_override: Some("model-pre-turn".to_owned()),
                add_attachments: vec![coclai_plugin_core::HookAttachment::Skill {
                    name: "probe".to_owned(),
                    path: existing_path.clone(),
                }],
                metadata_delta: json!({"from_pre_turn": 1}),
            },
        ),
    ];

    let metadata_events = Arc::new(Mutex::new(Vec::<(HookPhase, Value)>::new()));
    let hooks = RuntimeHookConfig::new()
        .with_pre_hook(Arc::new(PhasePatchPreHook {
            name: "phase_patch",
            patches,
        }))
        .with_post_hook(Arc::new(MetadataCapturePostHook {
            name: "metadata_capture",
            metadata: metadata_events.clone(),
        }));
    let runtime = spawn_run_prompt_mutation_probe_runtime(hooks).await;

    let result = runtime
        .run_prompt(PromptRunParams::new("/tmp", "original prompt"))
        .await
        .expect("run prompt");
    let payload: Value =
        serde_json::from_str(&result.assistant_text).expect("decode probe payload");
    assert_eq!(payload["threadModel"], json!("model-pre-run"));
    assert_eq!(payload["turnModel"], json!("model-pre-turn"));
    assert_eq!(payload["text"], json!("patched-in-pre-turn"));
    assert_eq!(payload["itemTypes"], json!(["text", "image", "skill"]),);

    let post_turn_metadata = {
        let captured = metadata_events.lock().expect("metadata lock");
        captured
            .iter()
            .find(|(phase, _)| *phase == HookPhase::PostTurn)
            .map(|(_, metadata)| metadata.clone())
            .expect("post-turn metadata")
    };
    assert_eq!(post_turn_metadata["from_pre_run"], json!(true));
    assert_eq!(post_turn_metadata["from_pre_turn"], json!(1));

    assert!(
        runtime.hook_report_snapshot().is_clean(),
        "valid mutations should not produce issues"
    );
    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn run_prompt_ignores_invalid_hook_attachment_with_fail_open() {
    let patches = vec![(
        HookPhase::PreTurn,
        coclai_plugin_core::HookPatch {
            prompt_override: None,
            model_override: None,
            add_attachments: vec![coclai_plugin_core::HookAttachment::LocalImage {
                path: "definitely_missing_image_for_hook_test.png".to_owned(),
            }],
            metadata_delta: Value::Null,
        },
    )];
    let hooks = RuntimeHookConfig::new().with_pre_hook(Arc::new(PhasePatchPreHook {
        name: "bad_attachment_patch",
        patches,
    }));
    let runtime = spawn_run_prompt_mutation_probe_runtime(hooks).await;

    let result = runtime
        .run_prompt(PromptRunParams::new("/tmp", "prompt"))
        .await
        .expect("main run should continue");
    let payload: Value =
        serde_json::from_str(&result.assistant_text).expect("decode probe payload");
    assert_eq!(payload["itemTypes"], json!(["text"]));
    let report = runtime.hook_report_snapshot();
    assert_eq!(report.issues.len(), 1);
    assert_eq!(report.issues[0].hook_name, "bad_attachment_patch");
    assert_eq!(report.issues[0].class, HookIssueClass::Validation);

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn pre_session_mutation_restricts_prompt_and_attachments_but_allows_model_and_metadata() {
    let patches = vec![(
        HookPhase::PreSessionStart,
        coclai_plugin_core::HookPatch {
            prompt_override: Some("not-allowed".to_owned()),
            model_override: Some("model-from-session-hook".to_owned()),
            add_attachments: vec![coclai_plugin_core::HookAttachment::ImageUrl {
                url: "https://example.com/ignored.png".to_owned(),
            }],
            metadata_delta: json!({"session_key": "session_value"}),
        },
    )];
    let metadata_events = Arc::new(Mutex::new(Vec::<(HookPhase, Value)>::new()));
    let hooks = RuntimeHookConfig::new()
        .with_pre_hook(Arc::new(PhasePatchPreHook {
            name: "session_patch",
            patches,
        }))
        .with_post_hook(Arc::new(MetadataCapturePostHook {
            name: "session_metadata_capture",
            metadata: metadata_events.clone(),
        }));
    let cfg = RuntimeConfig::new(
        python_session_mutation_probe_process(),
        workspace_schema_guard(),
    )
    .with_hooks(hooks);
    let runtime = Runtime::spawn_local(cfg).await.expect("spawn runtime");

    let thread = runtime
        .thread_start(ThreadStartParams::default())
        .await
        .expect("thread start");
    assert_eq!(thread.thread_id, "thr_model-from-session-hook");

    let report = runtime.hook_report_snapshot();
    assert_eq!(report.issues.len(), 2);
    assert!(report
        .issues
        .iter()
        .all(|issue| issue.class == HookIssueClass::Validation));
    assert_eq!(report.issues[0].phase, HookPhase::PreSessionStart);
    assert_eq!(report.issues[1].phase, HookPhase::PreSessionStart);

    let post_session_metadata = {
        let captured = metadata_events.lock().expect("metadata lock");
        captured
            .iter()
            .find(|(phase, _)| *phase == HookPhase::PostSessionStart)
            .map(|(_, metadata)| metadata.clone())
            .expect("post-session metadata")
    };
    assert_eq!(post_session_metadata["session_key"], json!("session_value"));

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn run_prompt_ignores_cross_thread_events_for_same_turn_id() {
    let runtime = spawn_run_prompt_cross_thread_noise_runtime().await;
    let result = runtime
        .run_prompt(PromptRunParams::new("/tmp", "say ok"))
        .await
        .expect("run prompt");

    assert_eq!(result.thread_id, "thr_prompt");
    assert_eq!(result.turn_id, "turn_prompt");
    assert_eq!(result.assistant_text, "ok-from-run-prompt");

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn run_prompt_simple_sends_default_effort() {
    let runtime = spawn_run_prompt_effort_probe_runtime().await;
    let result = runtime
        .run_prompt_simple("/tmp", "probe effort")
        .await
        .expect("run prompt simple");

    assert_eq!(result.assistant_text, "medium");

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn run_prompt_preserves_explicit_effort() {
    let runtime = spawn_run_prompt_effort_probe_runtime().await;
    let result = runtime
        .run_prompt(PromptRunParams {
            cwd: "/tmp".to_owned(),
            prompt: "probe effort".to_owned(),
            model: Some("gpt-5-codex".to_owned()),
            effort: Some(ReasoningEffort::High),
            approval_policy: ApprovalPolicy::Never,
            sandbox_policy: SandboxPolicy::Preset(SandboxPreset::ReadOnly),
            privileged_escalation_approved: false,
            attachments: vec![],
            timeout: Duration::from_secs(2),
        })
        .await
        .expect("run prompt");

    assert_eq!(result.assistant_text, "high");

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn run_prompt_surfaces_turn_error_when_text_is_empty() {
    let runtime = spawn_run_prompt_error_runtime().await;
    let err = runtime
        .run_prompt(PromptRunParams {
            cwd: "/tmp".to_owned(),
            prompt: "say ok".to_owned(),
            model: Some("gpt-5-codex".to_owned()),
            effort: Some(ReasoningEffort::High),
            approval_policy: ApprovalPolicy::Never,
            sandbox_policy: SandboxPolicy::Preset(SandboxPreset::ReadOnly),
            privileged_escalation_approved: false,
            attachments: vec![],
            timeout: Duration::from_secs(2),
        })
        .await
        .expect_err("run prompt must fail");

    match err {
        PromptRunError::TurnCompletedWithoutAssistantText(failure) => {
            assert_eq!(
                failure.terminal_state,
                PromptTurnTerminalState::CompletedWithoutAssistantText
            );
            assert_eq!(failure.source_method, "error");
            assert_eq!(failure.code, None);
            assert_eq!(failure.message, "model unavailable");
        }
        other => panic!("unexpected error: {other:?}"),
    }

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn run_prompt_surfaces_turn_failed_with_context() {
    let runtime = spawn_run_prompt_turn_failed_runtime().await;
    let err = runtime
        .run_prompt(PromptRunParams {
            cwd: "/tmp".to_owned(),
            prompt: "say ok".to_owned(),
            model: Some("gpt-5-codex".to_owned()),
            effort: Some(ReasoningEffort::High),
            approval_policy: ApprovalPolicy::Never,
            sandbox_policy: SandboxPolicy::Preset(SandboxPreset::ReadOnly),
            privileged_escalation_approved: false,
            attachments: vec![],
            timeout: Duration::from_secs(2),
        })
        .await
        .expect_err("run prompt must fail");

    match err {
        PromptRunError::TurnFailedWithContext(failure) => {
            assert_eq!(failure.terminal_state, PromptTurnTerminalState::Failed);
            assert_eq!(failure.source_method, "turn/failed");
            assert_eq!(failure.code, Some(429));
            assert_eq!(failure.message, "rate limited");
        }
        other => panic!("unexpected error: {other:?}"),
    }

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn run_prompt_timeout_uses_absolute_deadline_under_streaming_deltas() {
    let runtime = spawn_run_prompt_streaming_timeout_runtime().await;
    let timeout_value = Duration::from_millis(120);

    let started = Instant::now();
    let err = runtime
        .run_prompt(PromptRunParams::new("/tmp", "timeout probe").with_timeout(timeout_value))
        .await
        .expect_err("run prompt must timeout");

    assert!(matches!(err, PromptRunError::Timeout(d) if d == timeout_value));
    assert!(
        started.elapsed() < Duration::from_millis(350),
        "run_prompt exceeded expected absolute timeout window: {:?}",
        started.elapsed()
    );

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn run_prompt_recovers_when_live_stream_lags_past_terminal_event() {
    let runtime = spawn_run_prompt_lagged_completion_runtime().await;

    let result = runtime
        .run_prompt(PromptRunParams::new("/tmp", "lagged completion probe"))
        .await
        .expect("run prompt should recover from lagged stream");

    assert_eq!(result.thread_id, "thr_lagged");
    assert_eq!(result.turn_id, "turn_lagged");
    assert_eq!(result.assistant_text, "ok-from-thread-read");

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn run_prompt_timeout_emits_turn_interrupt_request() {
    let runtime = spawn_run_prompt_interrupt_probe_runtime().await;
    let mut live_rx = runtime.subscribe_live();
    let timeout_value = Duration::from_millis(120);

    let err = runtime
        .run_prompt(PromptRunParams::new("/tmp", "interrupt probe").with_timeout(timeout_value))
        .await
        .expect_err("run prompt must timeout");
    assert!(matches!(err, PromptRunError::Timeout(d) if d == timeout_value));

    let mut saw_interrupt = false;
    for _ in 0..16 {
        let envelope = tokio::time::timeout(Duration::from_secs(2), live_rx.recv())
            .await
            .expect("live timeout")
            .expect("live closed");
        if envelope.method.as_deref() == Some("probe/interruptSeen")
            && envelope.thread_id.as_deref() == Some("thr_interrupt_probe")
            && envelope.turn_id.as_deref() == Some("turn_interrupt_probe")
        {
            saw_interrupt = true;
            break;
        }
    }
    assert!(
        saw_interrupt,
        "timeout path must send turn/interrupt request"
    );

    runtime.shutdown().await.expect("shutdown");
}
