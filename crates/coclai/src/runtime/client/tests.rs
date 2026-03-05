use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::{
    ensure_session_open_for_prompt, ensure_session_open_for_rpc, parse_initialize_user_agent,
    profile_to_prompt_params, session_prompt_params, ClientConfig, CompatibilityGuard, RunProfile,
    SemVerTriplet, SessionConfig,
};
use crate::plugin::{HookAction, HookContext, HookIssue, HookPhase, PostHook, PreHook};
use crate::runtime::api::{
    ApprovalPolicy, PromptAttachment, ReasoningEffort, SandboxPolicy, SandboxPreset,
};
use crate::runtime::hooks::RuntimeHookConfig;

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

fn write_mock_cli_script(root: &std::path::Path) -> PathBuf {
    let path = root.join("mock_codex_cli.py");
    let script = r#"#!/usr/bin/env python3
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
        sys.stdout.write(json.dumps({
            "id": rpc_id,
            "result": {"ready": True, "userAgent": "Codex Desktop/0.104.0"}
        }) + "\n")
        sys.stdout.flush()
        continue

    if method == "thread/start":
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"thread": {"id": "thr_client"}}}) + "\n")
        sys.stdout.flush()
        continue

    if method == "thread/resume":
        thread_id = params.get("threadId") or "thr_client"
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"thread": {"id": thread_id}}}) + "\n")
        sys.stdout.flush()
        continue

    if method == "turn/start":
        thread_id = params.get("threadId") or "thr_client"
        turn_id = "turn_client"
        input_items = params.get("input") or []
        text = "ok"
        if len(input_items) > 0 and isinstance(input_items[0], dict):
            text = input_items[0].get("text") or "ok"

        sys.stdout.write(json.dumps({"method":"turn/started","params":{"threadId":thread_id,"turnId":turn_id}}) + "\n")
        sys.stdout.write(json.dumps({"method":"item/started","params":{"threadId":thread_id,"turnId":turn_id,"itemId":"item_1","itemType":"agentMessage"}}) + "\n")
        sys.stdout.write(json.dumps({"method":"item/agentMessage/delta","params":{"threadId":thread_id,"turnId":turn_id,"itemId":"item_1","delta":text}}) + "\n")
        sys.stdout.write(json.dumps({"method":"turn/completed","params":{"threadId":thread_id,"turnId":turn_id}}) + "\n")
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"turn": {"id": turn_id}}}) + "\n")
        sys.stdout.flush()
        continue

    if method == "thread/archive":
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"ok": True}}) + "\n")
        sys.stdout.flush()
        continue

    if method == "turn/interrupt":
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"ok": True}}) + "\n")
        sys.stdout.flush()
        continue

    sys.stdout.write(json.dumps({"id": rpc_id, "result": {"ok": True}}) + "\n")
    sys.stdout.flush()
"#;
    fs::write(&path, script).expect("write mock cli");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&path).expect("script metadata").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms).expect("set script executable");
    }
    path
}

fn temp_cwd(temp: &TempDir) -> String {
    temp.root.to_string_lossy().to_string()
}

async fn connect_mock_client(prefix: &str, config: ClientConfig) -> (TempDir, super::Client) {
    let temp = TempDir::new(prefix);
    let cli = write_mock_cli_script(&temp.root);
    let client = super::Client::connect(config.with_cli_bin(cli))
        .await
        .expect("client connect");
    (temp, client)
}

#[derive(Clone)]
struct RecordingPreHook {
    name: &'static str,
    phases: Arc<Mutex<Vec<HookPhase>>>,
}

impl PreHook for RecordingPreHook {
    fn name(&self) -> &'static str {
        self.name
    }

    fn call<'a>(
        &'a self,
        ctx: &'a HookContext,
    ) -> crate::plugin::HookFuture<'a, Result<HookAction, HookIssue>> {
        let phases = Arc::clone(&self.phases);
        Box::pin(async move {
            phases.lock().expect("pre hook lock").push(ctx.phase);
            Ok(HookAction::Noop)
        })
    }
}

#[derive(Clone)]
struct RecordingPostHook {
    name: &'static str,
    phases: Arc<Mutex<Vec<HookPhase>>>,
}

impl PostHook for RecordingPostHook {
    fn name(&self) -> &'static str {
        self.name
    }

    fn call<'a>(
        &'a self,
        ctx: &'a HookContext,
    ) -> crate::plugin::HookFuture<'a, Result<(), HookIssue>> {
        let phases = Arc::clone(&self.phases);
        Box::pin(async move {
            phases.lock().expect("post hook lock").push(ctx.phase);
            Ok(())
        })
    }
}

fn seen_phase(phases: &Arc<Mutex<Vec<HookPhase>>>, target: HookPhase) -> bool {
    phases.lock().expect("phase lock").contains(&target)
}

fn count_phase(phases: &Arc<Mutex<Vec<HookPhase>>>, target: HookPhase) -> usize {
    phases
        .lock()
        .expect("phase lock")
        .iter()
        .filter(|phase| **phase == target)
        .count()
}

#[test]
fn config_builder_sets_fields() {
    let cfg = ClientConfig::new().with_cli_bin("/opt/homebrew/bin/cli");
    assert_eq!(
        cfg.cli_bin,
        std::path::PathBuf::from("/opt/homebrew/bin/cli")
    );
    assert_eq!(
        cfg.compatibility_guard,
        CompatibilityGuard {
            require_initialize_user_agent: true,
            min_codex_version: Some(SemVerTriplet::new(0, 104, 0)),
        }
    );
}

#[test]
fn disable_compatibility_guard_overrides_defaults() {
    let cfg = ClientConfig::new().without_compatibility_guard();
    assert_eq!(
        cfg.compatibility_guard,
        CompatibilityGuard {
            require_initialize_user_agent: false,
            min_codex_version: None,
        }
    );
}

#[test]
fn parse_initialize_user_agent_extracts_product_and_semver() {
    let parsed = parse_initialize_user_agent("Codex Desktop/0.104.0 (Mac OS 26.3.0; arm64)");
    assert_eq!(
        parsed,
        Some(("Codex Desktop".to_owned(), SemVerTriplet::new(0, 104, 0)))
    );
}

#[test]
fn parse_initialize_user_agent_rejects_invalid_format() {
    assert_eq!(parse_initialize_user_agent("Codex Desktop"), None);
    assert_eq!(parse_initialize_user_agent("Codex Desktop/x.y.z"), None);
}

#[test]
fn session_config_defaults_are_explicit() {
    let cfg = SessionConfig::new("/work");
    assert_eq!(cfg.cwd, "/work");
    assert_eq!(cfg.model, None);
    assert_eq!(cfg.effort, ReasoningEffort::Medium);
    assert_eq!(cfg.approval_policy, ApprovalPolicy::Never);
    assert_eq!(
        cfg.sandbox_policy,
        SandboxPolicy::Preset(SandboxPreset::ReadOnly)
    );
    assert!(!cfg.privileged_escalation_approved);
    assert_eq!(cfg.timeout, Duration::from_secs(120));
    assert!(cfg.attachments.is_empty());
}

#[test]
fn run_profile_defaults_are_explicit() {
    let profile = RunProfile::new();
    assert_eq!(profile.model, None);
    assert_eq!(profile.effort, ReasoningEffort::Medium);
    assert_eq!(profile.approval_policy, ApprovalPolicy::Never);
    assert_eq!(
        profile.sandbox_policy,
        SandboxPolicy::Preset(SandboxPreset::ReadOnly)
    );
    assert!(!profile.privileged_escalation_approved);
    assert_eq!(profile.timeout, Duration::from_secs(120));
    assert!(profile.attachments.is_empty());
}

#[test]
fn session_config_from_profile_maps_all_fields() {
    let profile = RunProfile::new()
        .with_model("gpt-5-codex")
        .with_effort(ReasoningEffort::High)
        .with_approval_policy(ApprovalPolicy::OnRequest)
        .with_sandbox_policy(SandboxPolicy::Preset(SandboxPreset::WorkspaceWrite {
            writable_roots: vec!["/work".to_owned()],
            network_access: false,
        }))
        .allow_privileged_escalation()
        .with_timeout(Duration::from_secs(33))
        .with_attachment(PromptAttachment::ImageUrl {
            url: "https://example.com/a.png".to_owned(),
        });

    let cfg = SessionConfig::from_profile("/work", profile.clone());
    assert_eq!(cfg.cwd, "/work");
    assert_eq!(cfg.model.as_deref(), Some("gpt-5-codex"));
    assert_eq!(cfg.effort, ReasoningEffort::High);
    assert_eq!(cfg.approval_policy, ApprovalPolicy::OnRequest);
    assert_eq!(
        cfg.sandbox_policy,
        SandboxPolicy::Preset(SandboxPreset::WorkspaceWrite {
            writable_roots: vec!["/work".to_owned()],
            network_access: false,
        })
    );
    assert!(cfg.privileged_escalation_approved);
    assert_eq!(cfg.timeout, Duration::from_secs(33));
    assert_eq!(
        cfg.attachments,
        vec![PromptAttachment::ImageUrl {
            url: "https://example.com/a.png".to_owned()
        }]
    );

    let restored = cfg.profile();
    assert_eq!(restored, profile);
}

#[test]
fn session_prompt_params_maps_config_and_prompt() {
    let cfg = SessionConfig::new("/work")
        .with_model("gpt-5-codex")
        .with_effort(ReasoningEffort::High)
        .with_approval_policy(ApprovalPolicy::OnRequest)
        .with_sandbox_policy(SandboxPolicy::Preset(SandboxPreset::WorkspaceWrite {
            writable_roots: vec!["/work".to_owned()],
            network_access: false,
        }))
        .allow_privileged_escalation()
        .with_timeout(Duration::from_secs(33))
        .with_attachment(PromptAttachment::ImageUrl {
            url: "https://example.com/a.png".to_owned(),
        });

    let params = session_prompt_params(&cfg, "hello");
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
    assert_eq!(params.timeout, Duration::from_secs(33));
    assert_eq!(
        params.attachments,
        vec![PromptAttachment::ImageUrl {
            url: "https://example.com/a.png".to_owned()
        }]
    );
}

#[test]
fn profile_to_prompt_params_maps_profile_and_input() {
    let profile = RunProfile::new()
        .with_model("gpt-5-codex")
        .with_effort(ReasoningEffort::Low)
        .with_approval_policy(ApprovalPolicy::OnFailure)
        .with_sandbox_policy(SandboxPolicy::Preset(SandboxPreset::WorkspaceWrite {
            writable_roots: vec!["/tmp/work".to_owned()],
            network_access: true,
        }))
        .allow_privileged_escalation()
        .with_timeout(Duration::from_secs(15))
        .attach_path("README.md");

    let params = profile_to_prompt_params("/tmp/work".to_owned(), "hello", profile);
    assert_eq!(params.cwd, "/tmp/work");
    assert_eq!(params.prompt, "hello");
    assert_eq!(params.model.as_deref(), Some("gpt-5-codex"));
    assert_eq!(params.effort, Some(ReasoningEffort::Low));
    assert_eq!(params.approval_policy, ApprovalPolicy::OnFailure);
    assert_eq!(
        params.sandbox_policy,
        SandboxPolicy::Preset(SandboxPreset::WorkspaceWrite {
            writable_roots: vec!["/tmp/work".to_owned()],
            network_access: true,
        })
    );
    assert!(params.privileged_escalation_approved);
    assert_eq!(params.timeout, Duration::from_secs(15));
    assert_eq!(
        params.attachments,
        vec![PromptAttachment::AtPath {
            path: "README.md".to_owned(),
            placeholder: None
        }]
    );
}

#[test]
fn session_open_guards_return_error_when_closed() {
    let prompt_err = ensure_session_open_for_prompt(true).expect_err("must fail");
    assert!(matches!(
        prompt_err,
        crate::runtime::api::PromptRunError::Rpc(crate::runtime::errors::RpcError::InvalidRequest(
            _
        ))
    ));

    let rpc_err = ensure_session_open_for_rpc(true).expect_err("must fail");
    assert!(matches!(
        rpc_err,
        crate::runtime::errors::RpcError::InvalidRequest(_)
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn session_close_keeps_local_handle_closed_when_archive_rpc_fails() {
    let (temp, client) =
        connect_mock_client("runtime_client_session_close_failure", ClientConfig::new()).await;

    let session = client
        .start_session(SessionConfig::new(temp_cwd(&temp)))
        .await
        .expect("start session");

    client.shutdown().await.expect("shutdown runtime");

    let err = session
        .close()
        .await
        .expect_err("close must fail after shutdown");
    assert!(matches!(
        err,
        crate::runtime::errors::RpcError::InvalidRequest(_)
    ));
    assert!(session.is_closed());

    let second = session
        .close()
        .await
        .expect_err("repeated close must return same cached error");
    assert_eq!(second, err);

    let ask_err = session
        .ask("must fail")
        .await
        .expect_err("session is closed");
    assert!(matches!(
        ask_err,
        crate::runtime::api::PromptRunError::Rpc(crate::runtime::errors::RpcError::InvalidRequest(
            _
        ))
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn client_config_hooks_execute_on_run_path() {
    let phases = Arc::new(Mutex::new(Vec::<HookPhase>::new()));

    let config = ClientConfig::new()
        .with_pre_hook(Arc::new(RecordingPreHook {
            name: "cfg_pre",
            phases: Arc::clone(&phases),
        }))
        .with_post_hook(Arc::new(RecordingPostHook {
            name: "cfg_post",
            phases: Arc::clone(&phases),
        }));
    let (temp, client) = connect_mock_client("runtime_client_hooks_cfg", config).await;

    let out = client
        .run(temp_cwd(&temp), "cfg-hook")
        .await
        .expect("run with cfg hook");
    assert_eq!(out.assistant_text, "cfg-hook");
    assert!(seen_phase(&phases, HookPhase::PreRun));
    assert!(seen_phase(&phases, HookPhase::PostRun));

    client.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn run_profile_hooks_register_and_execute() {
    let phases = Arc::new(Mutex::new(Vec::<HookPhase>::new()));

    let (temp, client) = connect_mock_client(
        "runtime_client_hooks_profile",
        ClientConfig::new().with_hooks(RuntimeHookConfig::default()),
    )
    .await;

    let profile = RunProfile::new()
        .with_pre_hook(Arc::new(RecordingPreHook {
            name: "profile_pre",
            phases: Arc::clone(&phases),
        }))
        .with_post_hook(Arc::new(RecordingPostHook {
            name: "profile_post",
            phases: Arc::clone(&phases),
        }));
    let out = client
        .run_with_profile(temp_cwd(&temp), "profile-hook", profile)
        .await
        .expect("run with profile");
    assert_eq!(out.assistant_text, "profile-hook");
    assert!(seen_phase(&phases, HookPhase::PreRun));
    assert!(seen_phase(&phases, HookPhase::PostRun));

    client.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn session_config_hooks_register_and_execute() {
    let phases = Arc::new(Mutex::new(Vec::<HookPhase>::new()));

    let (temp, client) = connect_mock_client(
        "runtime_client_hooks_session",
        ClientConfig::new().with_hooks(RuntimeHookConfig::default()),
    )
    .await;

    let session = client
        .start_session(
            SessionConfig::new(temp_cwd(&temp))
                .with_pre_hook(Arc::new(RecordingPreHook {
                    name: "session_pre",
                    phases: Arc::clone(&phases),
                }))
                .with_post_hook(Arc::new(RecordingPostHook {
                    name: "session_post",
                    phases: Arc::clone(&phases),
                })),
        )
        .await
        .expect("start session");
    assert!(seen_phase(&phases, HookPhase::PreSessionStart));
    assert!(seen_phase(&phases, HookPhase::PostSessionStart));

    let out = session.ask("session-hook").await.expect("ask");
    assert_eq!(out.assistant_text, "session-hook");
    assert!(seen_phase(&phases, HookPhase::PreTurn));
    assert!(seen_phase(&phases, HookPhase::PostTurn));

    session.close().await.expect("close session");
    client.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn run_profile_hooks_do_not_leak_to_subsequent_runs() {
    let phases = Arc::new(Mutex::new(Vec::<HookPhase>::new()));

    let (temp, client) = connect_mock_client(
        "runtime_client_hooks_no_leak_run",
        ClientConfig::new().with_hooks(RuntimeHookConfig::default()),
    )
    .await;

    let profile = RunProfile::new()
        .with_pre_hook(Arc::new(RecordingPreHook {
            name: "ephemeral_pre",
            phases: Arc::clone(&phases),
        }))
        .with_post_hook(Arc::new(RecordingPostHook {
            name: "ephemeral_post",
            phases: Arc::clone(&phases),
        }));

    let first = client
        .run_with_profile(temp_cwd(&temp), "first", profile)
        .await
        .expect("run with profile");
    assert_eq!(first.assistant_text, "first");
    assert_eq!(count_phase(&phases, HookPhase::PreRun), 1);
    assert_eq!(count_phase(&phases, HookPhase::PostRun), 1);

    let second = client
        .run(temp_cwd(&temp), "second")
        .await
        .expect("run without profile");
    assert_eq!(second.assistant_text, "second");
    assert_eq!(
        count_phase(&phases, HookPhase::PreRun),
        1,
        "profile pre-hook leaked to subsequent run",
    );
    assert_eq!(
        count_phase(&phases, HookPhase::PostRun),
        1,
        "profile post-hook leaked to subsequent run",
    );

    client.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn session_hooks_do_not_leak_to_other_sessions() {
    let phases = Arc::new(Mutex::new(Vec::<HookPhase>::new()));

    let (temp, client) = connect_mock_client(
        "runtime_client_hooks_no_leak_session",
        ClientConfig::new().with_hooks(RuntimeHookConfig::default()),
    )
    .await;

    let session_a = client
        .start_session(
            SessionConfig::new(temp_cwd(&temp))
                .with_pre_hook(Arc::new(RecordingPreHook {
                    name: "session_a_pre",
                    phases: Arc::clone(&phases),
                }))
                .with_post_hook(Arc::new(RecordingPostHook {
                    name: "session_a_post",
                    phases: Arc::clone(&phases),
                })),
        )
        .await
        .expect("start session a");
    session_a.ask("first").await.expect("session a ask");
    session_a.close().await.expect("close session a");

    let baseline = phases.lock().expect("phase lock").len();

    let session_b = client
        .start_session(SessionConfig::new(temp_cwd(&temp)))
        .await
        .expect("start session b");
    session_b.ask("second").await.expect("session b ask");
    session_b.close().await.expect("close session b");

    let after = phases.lock().expect("phase lock").len();
    assert_eq!(
        baseline, after,
        "session hooks leaked into later session operations",
    );

    client.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn deprecated_client_session_aliases_remain_operational() {
    let (temp, client) =
        connect_mock_client("runtime_client_deprecated_aliases", ClientConfig::new()).await;

    #[allow(deprecated)]
    let session = client.setup(temp_cwd(&temp)).await.expect("setup");

    #[allow(deprecated)]
    let continued = client
        .continue_session(&session.thread_id, temp_cwd(&temp), "legacy-continue")
        .await
        .expect("continue_session");
    assert_eq!(continued.assistant_text, "legacy-continue");

    #[allow(deprecated)]
    let continued_with = client
        .continue_session_with(
            &session.thread_id,
            crate::runtime::api::PromptRunParams::new(temp_cwd(&temp), "legacy-with"),
        )
        .await
        .expect("continue_session_with");
    assert_eq!(continued_with.assistant_text, "legacy-with");

    #[allow(deprecated)]
    let continued_with_profile = client
        .continue_session_with_profile(
            &session.thread_id,
            temp_cwd(&temp),
            "legacy-profile",
            RunProfile::new().with_effort(ReasoningEffort::Low),
        )
        .await
        .expect("continue_session_with_profile");
    assert_eq!(continued_with_profile.assistant_text, "legacy-profile");

    #[allow(deprecated)]
    client
        .interrupt_session_turn(&session.thread_id, &continued.turn_id)
        .await
        .expect("interrupt_session_turn");

    #[allow(deprecated)]
    client
        .close_session(&session.thread_id)
        .await
        .expect("close_session");

    #[allow(deprecated)]
    let profiled_session = client
        .setup_with_profile(temp_cwd(&temp), RunProfile::new().with_model("gpt-5-codex"))
        .await
        .expect("setup_with_profile");
    profiled_session
        .close()
        .await
        .expect("close profiled session");

    client.shutdown().await.expect("shutdown");
}
