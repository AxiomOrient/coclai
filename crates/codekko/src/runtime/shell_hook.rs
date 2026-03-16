//! Shell-command hook adapter.
//!
//! Wraps an external process as a [`PreHook`] or [`PostHook`].
//! The process receives [`HookContext`] as JSON on stdin and signals its
//! decision via exit code + stdout JSON.
//!
//! ## Exit-code contract (PreHook)
//!
//! | exit | stdout | result |
//! |------|--------|--------|
//! | `0`  | `{}` or `{"action":"noop"}` | `HookAction::Noop` |
//! | `0`  | `{"action":"mutate", ...}` | `HookAction::Mutate(patch)` |
//! | `2`  | `{"message":"..."}` or plain text | `HookAction::Block(reason)` |
//! | any other | — | `Err(HookIssue { class: Execution })` |
//!
//! ## Exit-code contract (PostHook)
//!
//! | exit | result |
//! |------|--------|
//! | `0`  | `Ok(())` |
//! | any other | `Err(HookIssue { class: Execution })` |

use std::collections::HashMap;
use std::time::Duration;

use serde::Deserialize;
use serde_json::Value;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::plugin::{
    BlockReason, HookAction, HookAttachment, HookContext, HookFuture, HookIssue, HookIssueClass,
    HookPatch, HookPhase, PostHook, PreHook,
};

/// An external shell command registered as a [`PreHook`] or [`PostHook`].
///
/// Allocation: two Strings + optional env map at construction.
pub struct ShellCommandHook {
    name: &'static str,
    /// Shell command string passed to `sh -c`. Example: `"python3 /path/hook.py"`.
    command: String,
    /// Hard wall-clock limit for the subprocess. Default: 5 seconds.
    timeout: Duration,
    /// Extra environment variables injected into the subprocess.
    /// Complexity: O(n) insert at construction, O(n) clone per call.
    env: HashMap<String, String>,
}

impl ShellCommandHook {
    /// Construct with a 5-second default timeout.
    /// Allocation: two Strings.
    pub fn new(name: &'static str, command: impl Into<String>) -> Self {
        Self {
            name,
            command: command.into(),
            timeout: Duration::from_secs(5),
            env: HashMap::new(),
        }
    }

    /// Override the subprocess timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Inject one extra environment variable.
    /// Allocation: two Strings per call.
    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }
}

// ── Shared subprocess execution ──────────────────────────────────────────────

/// Raw output from a finished subprocess.
/// Allocation: two Strings (stdout, stderr).
struct ShellOutput {
    exit_code: i32,
    stdout: String,
}

/// Spawn `sh -c <command>`, feed `stdin_bytes` to stdin, collect stdout.
/// Allocation: stdin bytes cloned to pipe, stdout accumulated in String.
/// Side effect: spawns an OS process.
async fn run_process(
    command: &str,
    env: &HashMap<String, String>,
    stdin_bytes: Vec<u8>,
) -> Result<ShellOutput, String> {
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(command)
        .envs(env)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("spawn failed: {e}"))?;

    if let Some(mut stdin) = child.stdin.take() {
        // Ignore write errors — the process may have exited before reading all input.
        let _ = stdin.write_all(&stdin_bytes).await;
    }

    let output = child
        .wait_with_output()
        .await
        .map_err(|e| format!("wait failed: {e}"))?;

    Ok(ShellOutput {
        exit_code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
    })
}

// ── PreHook output parsing (pure functions) ───────────────────────────────────

/// Wire shape of a `mutate` response from a shell pre-hook.
/// Fields mirror [`HookPatch`] with camelCase naming for shell-script ergonomics.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ShellPreOutput {
    /// `"noop"` or `"mutate"`. Anything else is treated as `"noop"`.
    #[serde(default)]
    action: String,
    prompt_override: Option<String>,
    model_override: Option<String>,
    #[serde(default)]
    add_attachments: Vec<HookAttachment>,
    #[serde(default)]
    metadata_delta: Value,
}

/// Wire shape of a block response (exit 2).
#[derive(Deserialize, Default)]
struct ShellBlockOutput {
    #[serde(default)]
    message: String,
}

/// Parse stdout from a shell pre-hook that exited with code 0.
/// Pure: no I/O. Allocation: depends on patch contents.
fn parse_pre_output(
    hook_name: &str,
    phase: HookPhase,
    stdout: &str,
) -> Result<HookAction, HookIssue> {
    let trimmed = stdout.trim();
    // Empty stdout → Noop. Avoids requiring shell scripts to emit `{}`.
    if trimmed.is_empty() {
        return Ok(HookAction::Noop);
    }
    let parsed: ShellPreOutput = match serde_json::from_str(trimmed) {
        Ok(v) => v,
        Err(e) => {
            return Err(HookIssue {
                hook_name: hook_name.to_owned(),
                phase,
                class: HookIssueClass::Execution,
                message: format!("stdout parse error: {e}"),
            })
        }
    };
    if parsed.action.eq_ignore_ascii_case("mutate") {
        Ok(HookAction::Mutate(HookPatch {
            prompt_override: parsed.prompt_override,
            model_override: parsed.model_override,
            add_attachments: parsed.add_attachments,
            metadata_delta: parsed.metadata_delta,
        }))
    } else {
        Ok(HookAction::Noop)
    }
}

/// Parse stdout from a shell pre-hook that exited with code 2.
/// Pure: no I/O. Allocation: one String for message.
fn parse_block_output(hook_name: &str, phase: HookPhase, stdout: &str) -> BlockReason {
    let trimmed = stdout.trim();
    let message = if trimmed.is_empty() {
        "blocked by hook (no message)".to_owned()
    } else {
        // Try JSON `{"message":"..."}`, fall back to raw stdout as message.
        serde_json::from_str::<ShellBlockOutput>(trimmed)
            .map(|o| {
                if o.message.is_empty() {
                    trimmed.to_owned()
                } else {
                    o.message
                }
            })
            .unwrap_or_else(|_| trimmed.to_owned())
    };
    BlockReason {
        hook_name: hook_name.to_owned(),
        phase,
        message,
    }
}

/// Build an `Execution` issue for subprocess failures.
/// Pure. Allocation: one String.
fn execution_issue(hook_name: &str, phase: HookPhase, message: impl Into<String>) -> HookIssue {
    HookIssue {
        hook_name: hook_name.to_owned(),
        phase,
        class: HookIssueClass::Execution,
        message: message.into(),
    }
}

/// Build a `Timeout` issue.
/// Pure. Allocation: one String.
fn timeout_issue(hook_name: &str, phase: HookPhase, timeout: Duration) -> HookIssue {
    HookIssue {
        hook_name: hook_name.to_owned(),
        phase,
        class: HookIssueClass::Timeout,
        message: format!("shell hook timed out after {timeout:?}"),
    }
}

// ── PreHook impl ─────────────────────────────────────────────────────────────

impl PreHook for ShellCommandHook {
    fn name(&self) -> &'static str {
        self.name
    }

    fn call<'a>(&'a self, ctx: &'a HookContext) -> HookFuture<'a, Result<HookAction, HookIssue>> {
        Box::pin(async move {
            let stdin_bytes = match serde_json::to_vec(ctx) {
                Ok(b) => b,
                Err(e) => {
                    return Err(HookIssue {
                        hook_name: self.name.to_owned(),
                        phase: ctx.phase,
                        class: HookIssueClass::Internal,
                        message: format!("context serialize failed: {e}"),
                    })
                }
            };

            let output = match tokio::time::timeout(
                self.timeout,
                run_process(&self.command, &self.env, stdin_bytes),
            )
            .await
            {
                Err(_elapsed) => return Err(timeout_issue(self.name, ctx.phase, self.timeout)),
                Ok(Err(e)) => return Err(execution_issue(self.name, ctx.phase, e)),
                Ok(Ok(o)) => o,
            };

            match output.exit_code {
                0 => parse_pre_output(self.name, ctx.phase, &output.stdout),
                2 => Ok(HookAction::Block(parse_block_output(
                    self.name,
                    ctx.phase,
                    &output.stdout,
                ))),
                code => Err(execution_issue(
                    self.name,
                    ctx.phase,
                    format!("exited with code {code}"),
                )),
            }
        })
    }
}

// ── PostHook impl ─────────────────────────────────────────────────────────────

impl PostHook for ShellCommandHook {
    fn name(&self) -> &'static str {
        self.name
    }

    fn call<'a>(&'a self, ctx: &'a HookContext) -> HookFuture<'a, Result<(), HookIssue>> {
        Box::pin(async move {
            let stdin_bytes = match serde_json::to_vec(ctx) {
                Ok(b) => b,
                Err(e) => {
                    return Err(HookIssue {
                        hook_name: self.name.to_owned(),
                        phase: ctx.phase,
                        class: HookIssueClass::Internal,
                        message: format!("context serialize failed: {e}"),
                    })
                }
            };

            let output = match tokio::time::timeout(
                self.timeout,
                run_process(&self.command, &self.env, stdin_bytes),
            )
            .await
            {
                Err(_elapsed) => return Err(timeout_issue(self.name, ctx.phase, self.timeout)),
                Ok(Err(e)) => return Err(execution_issue(self.name, ctx.phase, e)),
                Ok(Ok(o)) => o,
            };

            if output.exit_code == 0 {
                Ok(())
            } else {
                Err(execution_issue(
                    self.name,
                    ctx.phase,
                    format!("exited with code {}", output.exit_code),
                ))
            }
        })
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn phase() -> HookPhase {
        HookPhase::PreRun
    }

    // ── parse_pre_output ────────────────────────────────────────────────────

    #[test]
    fn empty_stdout_is_noop() {
        assert_eq!(parse_pre_output("h", phase(), ""), Ok(HookAction::Noop));
        assert_eq!(parse_pre_output("h", phase(), "  "), Ok(HookAction::Noop));
    }

    #[test]
    fn empty_object_is_noop() {
        assert_eq!(parse_pre_output("h", phase(), "{}"), Ok(HookAction::Noop));
    }

    #[test]
    fn action_noop_explicit() {
        assert_eq!(
            parse_pre_output("h", phase(), r#"{"action":"noop"}"#),
            Ok(HookAction::Noop)
        );
    }

    #[test]
    fn action_mutate_model_override() {
        let out = parse_pre_output(
            "h",
            phase(),
            r#"{"action":"mutate","modelOverride":"claude-opus-4-6"}"#,
        );
        match out {
            Ok(HookAction::Mutate(patch)) => {
                assert_eq!(patch.model_override.as_deref(), Some("claude-opus-4-6"));
                assert!(patch.prompt_override.is_none());
            }
            other => panic!("expected Mutate, got {other:?}"),
        }
    }

    #[test]
    fn action_mutate_prompt_override() {
        let out = parse_pre_output(
            "h",
            phase(),
            r#"{"action":"mutate","promptOverride":"new prompt"}"#,
        );
        match out {
            Ok(HookAction::Mutate(patch)) => {
                assert_eq!(patch.prompt_override.as_deref(), Some("new prompt"));
            }
            other => panic!("expected Mutate, got {other:?}"),
        }
    }

    #[test]
    fn unknown_action_is_noop() {
        assert_eq!(
            parse_pre_output("h", phase(), r#"{"action":"unknown"}"#),
            Ok(HookAction::Noop)
        );
    }

    #[test]
    fn invalid_json_is_execution_issue() {
        let result = parse_pre_output("h", phase(), "not-json");
        assert!(matches!(
            result,
            Err(HookIssue {
                class: HookIssueClass::Execution,
                ..
            })
        ));
    }

    // ── parse_block_output ──────────────────────────────────────────────────

    #[test]
    fn block_with_json_message() {
        let r = parse_block_output("h", phase(), r#"{"message":"rm -rf blocked"}"#);
        assert_eq!(r.message, "rm -rf blocked");
        assert_eq!(r.hook_name, "h");
    }

    #[test]
    fn block_with_plain_text_message() {
        let r = parse_block_output("h", phase(), "plain text reason");
        assert_eq!(r.message, "plain text reason");
    }

    #[test]
    fn block_with_empty_stdout_gives_fallback() {
        let r = parse_block_output("h", phase(), "");
        assert_eq!(r.message, "blocked by hook (no message)");
    }

    #[test]
    fn block_with_json_empty_message_falls_back_to_raw() {
        // `{"message":""}` → raw stdout used as message
        let r = parse_block_output("h", phase(), r#"{"message":""}"#);
        assert_eq!(r.message, r#"{"message":""}"#);
    }

    // ── ShellCommandHook integration (requires sh) ──────────────────────────

    fn ctx() -> HookContext {
        use serde_json::json;
        HookContext {
            phase: HookPhase::PreRun,
            thread_id: None,
            turn_id: None,
            cwd: Some("/tmp".to_owned()),
            model: None,
            main_status: None,
            correlation_id: "hk-1".to_owned(),
            ts_ms: 0,
            metadata: json!({}),
            tool_name: None,
            tool_input: None,
        }
    }

    #[tokio::test]
    async fn pre_hook_exit0_empty_stdout_is_noop() {
        let hook = ShellCommandHook::new("test-noop", "exit 0");
        let result = PreHook::call(&hook, &ctx()).await;
        assert_eq!(result, Ok(HookAction::Noop));
    }

    #[tokio::test]
    async fn pre_hook_exit2_blocks() {
        let hook = ShellCommandHook::new("test-block", r#"echo '{"message":"denied"}' ; exit 2"#);
        let result = PreHook::call(&hook, &ctx()).await;
        match result {
            Ok(HookAction::Block(r)) => assert_eq!(r.message, "denied"),
            other => panic!("expected Block, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn pre_hook_exit1_is_execution_error() {
        let hook = ShellCommandHook::new("test-err", "exit 1");
        let result = PreHook::call(&hook, &ctx()).await;
        assert!(matches!(
            result,
            Err(HookIssue {
                class: HookIssueClass::Execution,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn pre_hook_exit0_mutate_model() {
        let hook = ShellCommandHook::new(
            "test-mutate",
            r#"echo '{"action":"mutate","modelOverride":"claude-haiku-4-5-20251001"}'"#,
        );
        let result = PreHook::call(&hook, &ctx()).await;
        match result {
            Ok(HookAction::Mutate(patch)) => {
                assert_eq!(
                    patch.model_override.as_deref(),
                    Some("claude-haiku-4-5-20251001")
                );
            }
            other => panic!("expected Mutate, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn pre_hook_timeout_returns_timeout_issue() {
        let hook = ShellCommandHook::new("test-timeout", "sleep 60")
            .with_timeout(Duration::from_millis(50));
        let result = PreHook::call(&hook, &ctx()).await;
        assert!(matches!(
            result,
            Err(HookIssue {
                class: HookIssueClass::Timeout,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn post_hook_exit0_is_ok() {
        let hook = ShellCommandHook::new("test-post", "exit 0");
        let result = PostHook::call(&hook, &ctx()).await;
        assert_eq!(result, Ok(()));
    }

    #[tokio::test]
    async fn post_hook_nonzero_is_execution_error() {
        let hook = ShellCommandHook::new("test-post-err", "exit 1");
        let result = PostHook::call(&hook, &ctx()).await;
        assert!(matches!(
            result,
            Err(HookIssue {
                class: HookIssueClass::Execution,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn stdin_receives_hook_context_json() {
        // The hook reads stdin and echoes the phase field back in model_override.
        // Uses `jq` if available; skip if not.
        if std::process::Command::new("jq")
            .arg("--version")
            .output()
            .is_err()
        {
            return; // jq not installed — skip
        }
        let hook = ShellCommandHook::new(
            "test-stdin",
            r#"phase=$(cat | jq -r '.phase'); echo "{\"action\":\"mutate\",\"modelOverride\":\"$phase\"}""#,
        );
        let result = PreHook::call(&hook, &ctx()).await;
        match result {
            Ok(HookAction::Mutate(patch)) => {
                assert_eq!(patch.model_override.as_deref(), Some("PreRun"));
            }
            other => panic!("expected Mutate, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn with_env_passes_env_to_process() {
        let hook = ShellCommandHook::new(
            "test-env",
            r#"echo "{\"action\":\"mutate\",\"modelOverride\":\"$MY_VAR\"}""#,
        )
        .with_env("MY_VAR", "injected-value");
        let result = PreHook::call(&hook, &ctx()).await;
        match result {
            Ok(HookAction::Mutate(patch)) => {
                assert_eq!(patch.model_override.as_deref(), Some("injected-value"));
            }
            other => panic!("expected Mutate, got {other:?}"),
        }
    }
}
