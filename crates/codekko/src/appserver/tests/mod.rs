use crate::runtime::turn_output::parse_thread_id;
use crate::runtime::{ThreadReadParams, ThreadReadResponse};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

use super::*;

#[derive(Debug)]
struct TempDir {
    root: PathBuf,
}

impl TempDir {
    fn new(prefix: &str) -> Self {
        let root = std::env::temp_dir().join(format!("{prefix}_{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).expect("create temp root");
        Self { root }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[derive(Clone)]
struct MockAppServer {
    app: AppServer,
    _temp: std::sync::Arc<TempDir>,
}

impl std::ops::Deref for MockAppServer {
    type Target = AppServer;

    fn deref(&self) -> &Self::Target {
        &self.app
    }
}

fn write_mock_cli_script(root: &Path) -> PathBuf {
    let path = root.join("mock_codex_appserver_cli.py");
    let script = r#"#!/usr/bin/env python3
import json
import sys

def make_thread(thread_id):
    return {
        "id": thread_id,
        "cliVersion": "0.104.0",
        "createdAt": 1700000000,
        "cwd": ".",
        "modelProvider": "openai",
        "path": f"/tmp/threads/{thread_id}.jsonl",
        "preview": "mock",
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
        out = {
            "id": rpc_id,
            "result": {"ready": True, "userAgent": "Codex Desktop/0.104.0"},
        }
        sys.stdout.write(json.dumps(out) + "\n")
        sys.stdout.flush()
        continue

    if rpc_id is None:
        continue

    if method == "thread/start":
        out = {"id": rpc_id, "result": {"thread": {"id": "thr_appserver"}}}
    elif method == "skills/list":
        cwd = (params.get("cwds") or ["."])[0]
        out = {
            "id": rpc_id,
            "result": {
                "data": [{
                    "cwd": cwd,
                    "skills": [{
                        "name": "skill-creator",
                        "description": "Create or update a Codex skill",
                        "path": f"{cwd}/.agents/skills/skill-creator/SKILL.md",
                        "scope": "repo",
                        "enabled": True
                    }],
                    "errors": []
                }]
            }
        }
    elif method == "command/exec":
        process_id = params.get("processId", "generated-proc")
        if params.get("streamStdoutStderr") or params.get("tty"):
            sys.stdout.write(json.dumps({
                "method": "command/exec/outputDelta",
                "params": {
                    "processId": process_id,
                    "stream": "stdout",
                    "deltaBase64": "c3RyZWFtLW91dA==",
                    "capReached": False
                }
            }) + "\n")
            sys.stdout.write(json.dumps({
                "method": "command/exec/outputDelta",
                "params": {
                    "processId": process_id,
                    "stream": "stderr",
                    "deltaBase64": "c3RyZWFtLWVycg==",
                    "capReached": False
                }
            }) + "\n")
            out = {"id": rpc_id, "result": {"exitCode": 0, "stdout": "", "stderr": ""}}
        else:
            out = {
                "id": rpc_id,
                "result": {"exitCode": 0, "stdout": "buffered-stdout", "stderr": "buffered-stderr"},
            }
    elif method == "command/exec/write":
        out = {"id": rpc_id, "result": {}}
    elif method == "command/exec/resize":
        out = {"id": rpc_id, "result": {}}
    elif method == "command/exec/terminate":
        out = {"id": rpc_id, "result": {}}
    elif method == "thread/read":
        thread_id = params.get("threadId", "thr_appserver")
        out = {"id": rpc_id, "result": {"thread": make_thread(thread_id)}}
    elif method == "thread/archive":
        out = {"id": rpc_id, "result": {"ok": True}}
    else:
        out = {"id": rpc_id, "result": {"ok": True}}

    sys.stdout.write(json.dumps(out) + "\n")
    sys.stdout.flush()
"#;

    fs::write(&path, script).expect("write mock appserver cli");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&path).expect("script metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms).expect("set script executable");
    }
    path
}

fn thread_start_params() -> Value {
    json!({
        "cwd": std::env::current_dir()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|_| ".".to_owned()),
        "approvalPolicy": "never",
        "sandbox": "read-only"
    })
}

async fn connect_real_appserver() -> MockAppServer {
    let temp = std::sync::Arc::new(TempDir::new("codekko_appserver_tests"));
    let cli = write_mock_cli_script(&temp.root);
    let app = AppServer::connect(
        ClientConfig::new()
            .with_cli_bin(cli)
            .without_compatibility_guard(),
    )
    .await
    .expect("connect mock codex app-server");
    MockAppServer { app, _temp: temp }
}

async fn start_thread(app: &AppServer) -> String {
    let response = app
        .request_json(methods::THREAD_START, thread_start_params())
        .await
        .expect("thread/start request");
    parse_thread_id(&response).expect("thread/start must return thread id")
}

async fn archive_thread_best_effort(app: &AppServer, thread_id: &str) {
    let _ = app
        .request_json(
            methods::THREAD_ARCHIVE,
            json!({
                "threadId": thread_id
            }),
        )
        .await;
}

// Unit: method constants and static contract surface.
mod contract;
// Contract: validation wrappers and known-method guards.
mod validated_calls;
// Integration: server-request channel ownership/wiring.
mod server_requests;
