use super::super::*;
use crate::appserver::{methods, AppServer};
use crate::runtime::turn_lifecycle::collect_turn_terminal_with_limits;
use crate::runtime::turn_output::{parse_thread_id, parse_turn_id, TurnStreamCollector};
use crate::runtime::PromptRunResult;
use crate::runtime::{Client, RunProfile, SessionConfig, ThreadReadParams, ThreadReadResponse};
use crate::ShellCommandHook;
use serde_json::json;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::{sleep, Duration as TokioDuration};
use uuid::Uuid;

const MAX_REAL_SERVER_RETRIES: usize = 5;
const QUICK_RUN_ATTEMPT_TIMEOUT: TokioDuration = TokioDuration::from_secs(45);
const WORKFLOW_RUN_ATTEMPT_TIMEOUT: TokioDuration = TokioDuration::from_secs(60);
const SESSION_RUN_ATTEMPT_TIMEOUT: TokioDuration = TokioDuration::from_secs(75);
const HOOK_RUN_ATTEMPT_TIMEOUT: TokioDuration = TokioDuration::from_secs(360);
const APPSERVER_ATTEMPT_TIMEOUT: TokioDuration = TokioDuration::from_secs(45);
const APPROVAL_ATTEMPT_TIMEOUT: TokioDuration = TokioDuration::from_secs(480);
const APPROVAL_REQUEST_TIMEOUT: TokioDuration = TokioDuration::from_secs(240);
const APPROVAL_COMPLETION_TIMEOUT: TokioDuration = TokioDuration::from_secs(240);
const APPROVAL_FILE_TEXT: &str = "approval-needed";
const REAL_SERVER_APPROVAL_ENV: &str = "CODEX_RUNTIME_REAL_SERVER_APPROVED";
const ATTACHED_DOC_TOKEN: &str = "sandboxPolicy";
const SESSION_MEMORY_TOKEN: &str = "AXIOM-742";
const RESUME_MEMORY_TOKEN: &str = "LATTICE-931";
const HOOK_FILE_TEXT: &str = "hook-wrote-this";

struct ScratchDirGuard {
    path: PathBuf,
}

impl ScratchDirGuard {
    fn new(label: &str) -> Result<Self, String> {
        let path = std::env::temp_dir().join(format!("codex-runtime-{label}-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&path)
            .map_err(|err| format!("failed to create scratch dir {}: {err}", path.display()))?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn utf8(&self) -> Result<String, String> {
        self.path
            .to_str()
            .map(ToOwned::to_owned)
            .ok_or_else(|| format!("scratch path is non-utf8: {}", self.path.display()))
    }
}

impl Drop for ScratchDirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn ensure_real_server_opt_in() -> Result<(), String> {
    match std::env::var(REAL_SERVER_APPROVAL_ENV) {
        Ok(v) if v == "1" => Ok(()),
        _ => Err(format!(
            "real-server test requires explicit approval: set {REAL_SERVER_APPROVAL_ENV}=1"
        )),
    }
}

fn current_dir_utf8() -> Result<String, String> {
    let cwd = std::env::current_dir().map_err(|err| format!("current_dir failed: {err}"))?;
    cwd.to_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| "current_dir is non-utf8; real-server test requires utf8 cwd".to_owned())
}

fn workspace_path_utf8(relative: &str) -> Result<String, String> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir
        .parent()
        .and_then(|path| path.parent())
        .ok_or_else(|| format!("failed to derive repo root from {}", manifest_dir.display()))?;
    let path = repo_root.join(relative);
    path.to_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| format!("workspace path is non-utf8: {}", path.display()))
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn write_shell_hook_probe(path: &Path, emit_noop_json: bool) -> Result<(), String> {
    let body = if emit_noop_json {
        r#"import json, os, sys
ctx = json.load(sys.stdin)
with open(os.environ["HOOK_LOG"], "a", encoding="utf-8") as handle:
    handle.write(json.dumps({
        "phase": ctx.get("phase"),
        "tool_name": ctx.get("tool_name"),
        "cwd": ctx.get("cwd")
    }) + "\n")
print("{}")
"#
    } else {
        r#"import json, os, sys
ctx = json.load(sys.stdin)
with open(os.environ["HOOK_LOG"], "a", encoding="utf-8") as handle:
    handle.write(json.dumps({
        "phase": ctx.get("phase"),
        "tool_name": ctx.get("tool_name"),
        "cwd": ctx.get("cwd")
    }) + "\n")
"#
    };
    std::fs::write(path, body)
        .map_err(|err| format!("failed to write hook probe {}: {err}", path.display()))
}

fn read_hook_log(path: &Path) -> Result<Vec<String>, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|err| format!("failed to read hook log {}: {err}", path.display()))?;
    Ok(text.lines().map(ToOwned::to_owned).collect())
}

async fn wait_for_exact_file_text(
    path: &Path,
    expected: &str,
    timeout: TokioDuration,
) -> Result<(), String> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match std::fs::read_to_string(path) {
            Ok(text) if text.trim() == expected => return Ok(()),
            Ok(_) | Err(_) => {
                if std::time::Instant::now() >= deadline {
                    break;
                }
                sleep(TokioDuration::from_millis(250)).await;
            }
        }
    }

    let detail = match std::fs::read_to_string(path) {
        Ok(text) => format!("unexpected file contents: {text}"),
        Err(err) => format!("file unavailable: {err}"),
    };
    Err(format!(
        "timed out waiting for {} to contain expected text: {detail}",
        path.display()
    ))
}

async fn shell_pre_and_post_hook_attempt() -> Result<(), String> {
    tokio::time::timeout(HOOK_RUN_ATTEMPT_TIMEOUT, async move {
        let scratch = ScratchDirGuard::new("live-hook-run")?;
        let cwd = scratch.utf8()?;
        let pre_log = scratch.path().join("pre.log");
        let post_log = scratch.path().join("post.log");
        let pre_script = scratch.path().join("pre_hook.py");
        let post_script = scratch.path().join("post_hook.py");
        write_shell_hook_probe(&pre_script, true)?;
        write_shell_hook_probe(&post_script, false)?;

        let pre_command = format!("python3 {}", shell_quote(&pre_script.display().to_string()));
        let post_command = format!(
            "python3 {}",
            shell_quote(&post_script.display().to_string())
        );

        let config = WorkflowConfig::new(cwd)
            .with_timeout(Duration::from_secs(300))
            .with_global_pre_hook(Arc::new(
                ShellCommandHook::new("live-pre", pre_command)
                    .with_env("HOOK_LOG", pre_log.display().to_string()),
            ))
            .with_global_post_hook(Arc::new(
                ShellCommandHook::new("live-post", post_command)
                    .with_env("HOOK_LOG", post_log.display().to_string()),
            ));
        let workflow = Workflow::connect(config)
            .await
            .map_err(|err| format!("workflow connect with hook probe failed: {err}"))?;
        let run_result = workflow.run("Reply with only OK.").await;
        let shutdown_result = workflow.shutdown().await;

        let out = match run_result {
            Ok(out) => out,
            Err(err) => return Err(format!("hook probe run failed: {err:?}")),
        };
        if let Err(err) = shutdown_result {
            return Err(format!("hook probe shutdown failed: {err}"));
        }
        assert_prompt_result_non_empty("hook probe run", &out)?;

        let pre_lines = read_hook_log(&pre_log)?;
        let post_lines = read_hook_log(&post_log)?;
        if !pre_lines.iter().any(|line| line.contains("PreRun")) {
            return Err(format!("pre hook log missing PreRun phase: {pre_lines:?}"));
        }
        if !post_lines.iter().any(|line| line.contains("PostRun")) {
            return Err(format!(
                "post hook log missing PostRun phase: {post_lines:?}"
            ));
        }

        Ok(())
    })
    .await
    .map_err(|_| format!("shell hook probe timed out after {HOOK_RUN_ATTEMPT_TIMEOUT:?}"))?
}

async fn pre_tool_use_hook_file_write_attempt() -> Result<(), String> {
    tokio::time::timeout(APPROVAL_ATTEMPT_TIMEOUT, async move {
        let scratch = ScratchDirGuard::new("live-tool-hook")?;
        let cwd = scratch.utf8()?;
        let file_path = scratch.path().join("hook_probe.txt");
        let tool_log = scratch.path().join("tool.log");
        let tool_script = scratch.path().join("tool_hook.py");
        write_shell_hook_probe(&tool_script, true)?;
        let tool_command = format!(
            "python3 {}",
            shell_quote(&tool_script.display().to_string())
        );

        let workflow = Workflow::connect(
            WorkflowConfig::new(cwd)
                .with_run_profile(
                    RunProfile::new()
                        .with_timeout(Duration::from_secs(300))
                        .with_approval_policy(crate::runtime::ApprovalPolicy::OnRequest)
                        .allow_privileged_escalation()
                        .with_sandbox_policy(crate::runtime::SandboxPolicy::Preset(
                            crate::runtime::SandboxPreset::ReadOnly,
                        )),
                )
                .with_global_pre_tool_use_hook(Arc::new(
                    ShellCommandHook::new("live-pre-tool", tool_command)
                        .with_env("HOOK_LOG", tool_log.display().to_string()),
                )),
        )
        .await
        .map_err(|err| format!("pre-tool-use workflow connect failed: {err}"))?;

        let prompt = format!(
            "Create the file {} with exact contents {} and then reply with only DONE.",
            file_path.display(),
            HOOK_FILE_TEXT
        );
        let out = workflow
            .run(prompt)
            .await
            .map_err(|err| format!("pre-tool-use hook run failed: {err:?}"))?;
        workflow
            .shutdown()
            .await
            .map_err(|err| format!("pre-tool-use workflow shutdown failed: {err}"))?;
        assert_prompt_result_non_empty("pre-tool-use hook run", &out)?;
        if !out.assistant_text.trim_end().ends_with("DONE") {
            return Err(format!(
                "pre-tool-use hook run returned unexpected assistant text: {}",
                out.assistant_text
            ));
        }

        let file_text = std::fs::read_to_string(&file_path)
            .map_err(|err| format!("pre-tool-use hook file read failed: {err}"))?;
        if file_text.trim() != HOOK_FILE_TEXT {
            return Err(format!(
                "pre-tool-use hook file write mismatch: {file_text}"
            ));
        }

        let tool_lines = read_hook_log(&tool_log)?;
        if !tool_lines.iter().any(|line| line.contains("PreToolUse")) {
            return Err(format!(
                "tool hook log missing PreToolUse phase: {tool_lines:?}"
            ));
        }

        Ok(())
    })
    .await
    .map_err(|_| {
        format!("pre-tool-use hook file write timed out after {APPROVAL_ATTEMPT_TIMEOUT:?}")
    })?
}

fn is_transient_real_server_error(err: &str) -> bool {
    let e = err.to_ascii_lowercase();
    e.contains("stream disconnected")
        || e.contains("error sending request")
        || e.contains("timed out")
        || e.contains("connection reset")
        || e.contains("connection refused")
}

async fn backoff_after_attempt(attempt: usize) {
    let seconds = (attempt as u64).min(5);
    sleep(TokioDuration::from_secs(seconds)).await;
}

async fn run_with_retries<T, Fut, F>(label: &str, mut operation: F) -> Result<T, String>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, String>>,
{
    let mut last_err = None;
    for attempt in 1..=MAX_REAL_SERVER_RETRIES {
        match operation().await {
            Ok(value) => return Ok(value),
            Err(err_text) => {
                last_err = Some(err_text.clone());
                if !is_transient_real_server_error(&err_text) || attempt == MAX_REAL_SERVER_RETRIES
                {
                    break;
                }
                backoff_after_attempt(attempt).await;
            }
        }
    }

    Err(format!(
        "{label} failed after retries({MAX_REAL_SERVER_RETRIES}): {:?}",
        last_err
    ))
}

fn assert_prompt_result_non_empty(label: &str, out: &PromptRunResult) -> Result<(), String> {
    if out.thread_id.is_empty() {
        return Err(format!("{label} returned empty thread_id"));
    }
    if out.turn_id.is_empty() {
        return Err(format!("{label} returned empty turn_id"));
    }
    if out.assistant_text.trim().is_empty() {
        return Err(format!("{label} returned empty assistant_text"));
    }
    Ok(())
}

async fn quick_run_attempt(cwd: String, prompt: &'static str) -> Result<PromptRunResult, String> {
    tokio::time::timeout(QUICK_RUN_ATTEMPT_TIMEOUT, quick_run(cwd, prompt))
        .await
        .map_err(|_| format!("quick_run attempt timed out after {QUICK_RUN_ATTEMPT_TIMEOUT:?}"))?
        .map_err(|err| format!("{err:?}"))
}

async fn quick_run_with_profile_attempt(
    cwd: String,
    prompt: &'static str,
    profile: RunProfile,
) -> Result<PromptRunResult, String> {
    tokio::time::timeout(
        QUICK_RUN_ATTEMPT_TIMEOUT,
        quick_run_with_profile(cwd, prompt, profile),
    )
    .await
    .map_err(|_| {
        format!("quick_run_with_profile attempt timed out after {QUICK_RUN_ATTEMPT_TIMEOUT:?}")
    })?
    .map_err(|err| format!("{err:?}"))
}

async fn workflow_run_attempt(cwd: String) -> Result<PromptRunResult, String> {
    tokio::time::timeout(WORKFLOW_RUN_ATTEMPT_TIMEOUT, async move {
        let config = WorkflowConfig::new(cwd).with_timeout(Duration::from_secs(120));
        let workflow = Workflow::connect(config)
            .await
            .map_err(|err| format!("workflow connect with real codex server failed: {err}"))?;
        let run_result = workflow
            .run("Reply with one short sentence about Rust testing.")
            .await;
        let shutdown_result = workflow.shutdown().await;

        match run_result {
            Ok(result) => {
                if let Err(shutdown_err) = shutdown_result {
                    return Err(format!(
                        "workflow shutdown failed after successful run: {shutdown_err}"
                    ));
                }
                Ok(result)
            }
            Err(err) => {
                if let Err(shutdown_err) = shutdown_result {
                    eprintln!("warning: shutdown failed after run error: {shutdown_err}");
                }
                Err(format!("{err:?}"))
            }
        }
    })
    .await
    .map_err(|_| format!("workflow attempt timed out after {WORKFLOW_RUN_ATTEMPT_TIMEOUT:?}"))?
}

async fn workflow_session_memory_attempt(
    cwd: String,
    token: &'static str,
) -> Result<PromptRunResult, String> {
    tokio::time::timeout(SESSION_RUN_ATTEMPT_TIMEOUT, async move {
        let config = WorkflowConfig::new(cwd).with_timeout(Duration::from_secs(120));
        let workflow = Workflow::connect(config)
            .await
            .map_err(|err| format!("workflow connect with real codex server failed: {err}"))?;
        let session = workflow
            .setup_session()
            .await
            .map_err(|err| format!("workflow setup_session failed: {err:?}"))?;

        let first_prompt =
            format!("Remember the exact token {token}. Reply with only ACK and nothing else.");
        let first_result = session
            .ask(first_prompt)
            .await
            .map_err(|err| format!("workflow session first ask failed: {err:?}"))?;
        assert_prompt_result_non_empty("workflow session first ask", &first_result)?;

        let second_result = session
            .ask(
                "Reply with only the exact token you were told to remember earlier in this thread.",
            )
            .await
            .map_err(|err| format!("workflow session second ask failed: {err:?}"))?;

        if let Err(err) = session.close().await {
            return Err(format!("workflow session close failed: {err}"));
        }
        if let Err(err) = workflow.shutdown().await {
            return Err(format!(
                "workflow shutdown failed after session flow: {err}"
            ));
        }

        Ok(second_result)
    })
    .await
    .map_err(|_| {
        format!("workflow session attempt timed out after {SESSION_RUN_ATTEMPT_TIMEOUT:?}")
    })?
}

async fn client_resume_session_attempt(
    cwd: String,
    token: &'static str,
) -> Result<PromptRunResult, String> {
    tokio::time::timeout(SESSION_RUN_ATTEMPT_TIMEOUT, async move {
        let client = Client::connect_default()
            .await
            .map_err(|err| format!("client connect with real codex server failed: {err}"))?;
        let config = SessionConfig::new(cwd).with_timeout(Duration::from_secs(120));
        let session = client
            .start_session(config.clone())
            .await
            .map_err(|err| format!("start_session failed: {err:?}"))?;
        let thread_id = session.thread_id.clone();

        let first_prompt =
            format!("Remember the exact token {token}. Reply with only ACK and nothing else.");
        let first_result = session
            .ask(first_prompt)
            .await
            .map_err(|err| format!("initial session ask failed: {err:?}"))?;
        assert_prompt_result_non_empty("initial session ask", &first_result)?;

        let resumed = client
            .resume_session(&thread_id, config)
            .await
            .map_err(|err| format!("resume_session failed: {err:?}"))?;
        let second_result = resumed
            .ask(
                "Reply with only the exact token you were told to remember earlier in this thread.",
            )
            .await
            .map_err(|err| format!("resumed session ask failed: {err:?}"))?;

        if let Err(err) = resumed.close().await {
            return Err(format!("resumed session close failed: {err}"));
        }
        if let Err(err) = client.shutdown().await {
            return Err(format!("client shutdown failed after resume flow: {err}"));
        }

        Ok(second_result)
    })
    .await
    .map_err(|_| {
        format!("resume_session attempt timed out after {SESSION_RUN_ATTEMPT_TIMEOUT:?}")
    })?
}

async fn appserver_roundtrip_attempt(cwd: String) -> Result<(), String> {
    tokio::time::timeout(APPSERVER_ATTEMPT_TIMEOUT, async move {
        let app = AppServer::connect_default()
            .await
            .map_err(|err| format!("appserver connect_default failed: {err}"))?;

        let response = app
            .request_json(
                methods::THREAD_START,
                json!({
                    "cwd": cwd,
                    "approvalPolicy": "never",
                    "sandbox": "read-only"
                }),
            )
            .await
            .map_err(|err| format!("appserver thread/start failed: {err}"))?;
        let thread_id = parse_thread_id(&response)
            .ok_or_else(|| format!("appserver thread/start missing thread id in result: {response}"))?;

        let read: ThreadReadResponse = app
            .request_typed(
                methods::THREAD_READ,
                ThreadReadParams {
                    thread_id: thread_id.clone(),
                    include_turns: Some(false),
                },
            )
            .await
            .map_err(|err| format!("appserver thread/read failed: {err}"))?;
        if read.thread.id != thread_id {
            return Err(format!(
                "appserver thread/read returned mismatched thread id: expected={thread_id} actual={}",
                read.thread.id
            ));
        }

        let _ = app
            .request_json(methods::THREAD_ARCHIVE, json!({ "threadId": thread_id }))
            .await;
        app.shutdown()
            .await
            .map_err(|err| format!("appserver shutdown failed: {err}"))?;
        Ok(())
    })
    .await
    .map_err(|_| format!("appserver roundtrip timed out after {APPSERVER_ATTEMPT_TIMEOUT:?}"))?
}

async fn appserver_approval_roundtrip_attempt() -> Result<(), String> {
    tokio::time::timeout(APPROVAL_ATTEMPT_TIMEOUT, async move {
        let scratch = ScratchDirGuard::new("live-approval")?;
        let cwd = scratch.utf8()?;
        let file_path = scratch.path().join("live_probe.txt");
        let file_path_utf8 = file_path
            .to_str()
            .map(ToOwned::to_owned)
            .ok_or_else(|| format!("approval target path is non-utf8: {}", file_path.display()))?;

        let app = AppServer::connect_default()
            .await
            .map_err(|err| format!("appserver connect_default failed: {err}"))?;
        let attempt = async {
            let mut live_rx = app.runtime().subscribe_live();
            let mut server_requests = app
                .take_server_requests()
                .await
                .map_err(|err| format!("take_server_requests failed: {err}"))?;
            let thread_response = app
                .request_json(
                    methods::THREAD_START,
                    json!({
                        "cwd": cwd.clone(),
                        "approvalPolicy": "on-request",
                        "sandbox": "read-only"
                    }),
                )
                .await
                .map_err(|err| format!("appserver approval thread/start failed: {err}"))?;
            let thread_id = parse_thread_id(&thread_response).ok_or_else(|| {
                format!("appserver approval thread/start missing thread id: {thread_response}")
            })?;

            let turn_response = app
                .request_json(
                    methods::TURN_START,
                    json!({
                        "threadId": thread_id.clone(),
                        "input": [{
                            "type": "text",
                            "text": format!(
                                "Create the file {file_path_utf8} with exact contents {APPROVAL_FILE_TEXT} and then reply with only DONE."
                            )
                        }]
                    }),
                )
                .await
                .map_err(|err| format!("appserver approval turn/start failed: {err}"))?;
            let turn_id = parse_turn_id(&turn_response).ok_or_else(|| {
                format!("appserver approval turn/start missing turn id: {turn_response}")
            })?;

            let request = tokio::time::timeout(APPROVAL_REQUEST_TIMEOUT, server_requests.recv())
                .await
                .map_err(|_| {
                    format!(
                        "appserver approval request timed out after {APPROVAL_REQUEST_TIMEOUT:?}"
                    )
                })?
                .ok_or_else(|| "server request channel closed before approval".to_owned())?;

            if request.method != "item/fileChange/requestApproval"
                && request.method != "item/commandExecution/requestApproval"
            {
                return Err(format!(
                    "unexpected server request method during approval scenario: {}",
                    request.method
                ));
            }

            app.respond_server_request_ok(&request.approval_id, json!({ "decision": "accept" }))
                .await
                .map_err(|err| format!("approval response failed: {err}"))?;

            wait_for_exact_file_text(&file_path, APPROVAL_FILE_TEXT, APPROVAL_COMPLETION_TIMEOUT)
                .await
                .map_err(|err| format!("approval scenario file write failed: {err}"))?;

            let read: ThreadReadResponse = app
                .request_typed(
                    methods::THREAD_READ,
                    ThreadReadParams {
                        thread_id: thread_id.clone(),
                        include_turns: Some(false),
                    },
                )
                .await
                .map_err(|err| format!("approval scenario thread/read failed: {err}"))?;
            if read.thread.id != thread_id {
                return Err(format!(
                    "approval scenario thread/read returned mismatched thread id: expected={thread_id} actual={}",
                    read.thread.id
                ));
            }

            // Best-effort completion drain only. The core low-level contract is
            // requestApproval -> respond -> side effect, not assistant wording latency.
            let mut stream = TurnStreamCollector::new(&thread_id, &turn_id);
            let _ = collect_turn_terminal_with_limits::<String, _, _, _>(
                &mut live_rx,
                &mut stream,
                2048,
                TokioDuration::from_secs(5),
                |_| Ok(()),
                |_| async { Ok(None) },
            )
            .await;

            Ok(())
        }
        .await;

        let shutdown = app
            .shutdown()
            .await
            .map_err(|err| format!("appserver shutdown failed after approval scenario: {err}"));

        match (attempt, shutdown) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(err), Ok(())) => Err(err),
            (Ok(()), Err(err)) => Err(err),
            (Err(err), Err(shutdown_err)) => {
                Err(format!("{err}; shutdown cleanup also failed: {shutdown_err}"))
            }
        }
    })
    .await
    .map_err(|_| format!("appserver approval roundtrip timed out after {APPROVAL_ATTEMPT_TIMEOUT:?}"))?
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "opt-in live test: requires real codex server"]
async fn quick_run_executes_prompt_against_real_codex_server() -> Result<(), String> {
    ensure_real_server_opt_in()?;
    let cwd = current_dir_utf8()?;
    let prompt = "Reply with one short sentence about this workspace.";
    let out = run_with_retries("quick_run with real codex server", || {
        quick_run_attempt(cwd.clone(), prompt)
    })
    .await?;
    assert_prompt_result_non_empty("real-server quick_run", &out)?;
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "opt-in live test: requires real codex server"]
async fn workflow_run_executes_prompt_against_real_codex_server() -> Result<(), String> {
    ensure_real_server_opt_in()?;
    let cwd = current_dir_utf8()?;
    let out = run_with_retries("workflow run against real codex server", || {
        workflow_run_attempt(cwd.clone())
    })
    .await?;
    assert_prompt_result_non_empty("real-server workflow run", &out)?;
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "opt-in live test: requires real codex server"]
async fn quick_run_with_profile_reads_attached_core_api_file_against_real_codex_server(
) -> Result<(), String> {
    ensure_real_server_opt_in()?;
    let cwd = current_dir_utf8()?;
    let plan_path = workspace_path_utf8("docs/API_REFERENCE.md")?;
    let profile = RunProfile::new()
        .attach_path(plan_path)
        .with_timeout(Duration::from_secs(120));
    let prompt =
        "Read the attached API document and reply with only the sandbox policy field name.";

    let out = run_with_retries("quick_run_with_profile attachment scenario", || {
        quick_run_with_profile_attempt(cwd.clone(), prompt, profile.clone())
    })
    .await?;
    assert_prompt_result_non_empty("real-server quick_run_with_profile", &out)?;
    if !contains_attached_doc_token(&out.assistant_text) {
        return Err(format!(
            "attachment scenario did not return expected API token {ATTACHED_DOC_TOKEN}: {}",
            out.assistant_text
        ));
    }
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "opt-in live test: requires real codex server"]
async fn workflow_session_preserves_context_against_real_codex_server() -> Result<(), String> {
    ensure_real_server_opt_in()?;
    let cwd = current_dir_utf8()?;
    let out = run_with_retries("workflow session memory scenario", || {
        workflow_session_memory_attempt(cwd.clone(), SESSION_MEMORY_TOKEN)
    })
    .await?;
    assert_prompt_result_non_empty("real-server workflow session", &out)?;
    if !out.assistant_text.contains(SESSION_MEMORY_TOKEN) {
        return Err(format!(
            "workflow session memory scenario lost token {SESSION_MEMORY_TOKEN}: {}",
            out.assistant_text
        ));
    }
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "opt-in live test: requires real codex server"]
async fn resume_session_preserves_context_against_real_codex_server() -> Result<(), String> {
    ensure_real_server_opt_in()?;
    let cwd = current_dir_utf8()?;
    let out = run_with_retries("resume_session memory scenario", || {
        client_resume_session_attempt(cwd.clone(), RESUME_MEMORY_TOKEN)
    })
    .await?;
    assert_prompt_result_non_empty("real-server resume_session", &out)?;
    if !out.assistant_text.contains(RESUME_MEMORY_TOKEN) {
        return Err(format!(
            "resume_session scenario lost token {RESUME_MEMORY_TOKEN}: {}",
            out.assistant_text
        ));
    }
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "opt-in live test: requires real codex server"]
async fn appserver_thread_roundtrip_executes_against_real_codex_server() -> Result<(), String> {
    ensure_real_server_opt_in()?;
    let cwd = current_dir_utf8()?;
    run_with_retries("appserver thread roundtrip", || {
        appserver_roundtrip_attempt(cwd.clone())
    })
    .await?;
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "opt-in live test: requires real codex server"]
async fn appserver_approval_roundtrip_executes_against_real_codex_server() -> Result<(), String> {
    ensure_real_server_opt_in()?;
    run_with_retries("appserver approval roundtrip", || async {
        appserver_approval_roundtrip_attempt().await
    })
    .await?;
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "opt-in live test: requires real codex server"]
async fn shell_pre_and_post_hooks_execute_against_real_codex_server() -> Result<(), String> {
    ensure_real_server_opt_in()?;
    run_with_retries("shell pre/post hook probe", || async {
        shell_pre_and_post_hook_attempt().await
    })
    .await?;
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "opt-in live test: requires real codex server"]
async fn pre_tool_use_hook_observes_file_write_against_real_codex_server() -> Result<(), String> {
    ensure_real_server_opt_in()?;
    run_with_retries("pre-tool-use hook file write probe", || async {
        pre_tool_use_hook_file_write_attempt().await
    })
    .await?;
    Ok(())
}

fn contains_attached_doc_token(text: &str) -> bool {
    let normalized: String = text
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(|ch| ch.to_lowercase())
        .collect();
    normalized.contains("sandboxpolicy")
}
