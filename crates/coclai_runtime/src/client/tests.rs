use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::{
    ensure_session_open_for_prompt, ensure_session_open_for_rpc, parse_initialize_user_agent,
    profile_to_prompt_params, resolve_default_schema_dir, session_prompt_params,
    validate_schema_dir, ClientConfig, ClientError, CompatibilityGuard, RunProfile, SemVerTriplet,
    SessionConfig, DEFAULT_SCHEMA_RELATIVE_DIR,
};
use crate::api::{ApprovalPolicy, PromptAttachment, ReasoningEffort, SandboxPolicy, SandboxPreset};
use crate::hooks::RuntimeHookConfig;
use coclai_plugin_core::{HookAction, HookContext, HookIssue, HookPhase, PostHook, PreHook};

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

fn workspace_schema_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../SCHEMAS/app-server/active")
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
    ) -> coclai_plugin_core::HookFuture<'a, Result<HookAction, HookIssue>> {
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
    ) -> coclai_plugin_core::HookFuture<'a, Result<(), HookIssue>> {
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
fn resolve_schema_dir_rejects_missing_path() {
    let err = validate_schema_dir(std::path::PathBuf::from("/not/found")).expect_err("must fail");
    assert!(matches!(err, ClientError::SchemaDirNotFound(_)));
}

#[test]
fn resolve_schema_dir_accepts_existing_directory() {
    let root = std::env::temp_dir().join(format!("client_test_{}", uuid::Uuid::new_v4()));
    let schema = root.join("schema");
    fs::create_dir_all(&schema).expect("create dir");

    let resolved = validate_schema_dir(schema.clone()).expect("resolve schema");
    assert_eq!(resolved, schema);

    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn resolve_default_schema_dir_prefers_cwd_default() {
    let root = std::env::temp_dir().join(format!("client_test_{}", uuid::Uuid::new_v4()));
    let cwd = root.join("cwd");
    let cwd_schema = cwd.join(DEFAULT_SCHEMA_RELATIVE_DIR);
    let package_schema = root.join("pkg-schema");
    fs::create_dir_all(&cwd_schema).expect("create cwd schema dir");
    fs::create_dir_all(&package_schema).expect("create package schema dir");

    let resolved = resolve_default_schema_dir(&cwd, &package_schema).expect("resolve schema");
    assert_eq!(resolved, cwd_schema);

    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn resolve_default_schema_dir_falls_back_to_package_default() {
    let root = std::env::temp_dir().join(format!("client_test_{}", uuid::Uuid::new_v4()));
    let cwd = root.join("cwd");
    let package_schema = root.join("pkg-schema");
    fs::create_dir_all(&cwd).expect("create cwd dir");
    fs::create_dir_all(&package_schema).expect("create package schema dir");

    let resolved = resolve_default_schema_dir(&cwd, &package_schema).expect("resolve schema");
    assert_eq!(resolved, package_schema);

    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn config_builder_sets_fields() {
    let cfg = ClientConfig::new()
        .with_cli_bin("/opt/homebrew/bin/cli")
        .with_schema_dir("/tmp/schema");
    assert_eq!(
        cfg.cli_bin,
        std::path::PathBuf::from("/opt/homebrew/bin/cli")
    );
    assert_eq!(
        cfg.schema_dir,
        Some(std::path::PathBuf::from("/tmp/schema"))
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
        crate::api::PromptRunError::Rpc(crate::errors::RpcError::InvalidRequest(_))
    ));

    let rpc_err = ensure_session_open_for_rpc(true).expect_err("must fail");
    assert!(matches!(
        rpc_err,
        crate::errors::RpcError::InvalidRequest(_)
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn client_config_hooks_execute_on_run_path() {
    let temp = TempDir::new("coclai_client_hooks_cfg");
    let cli = write_mock_cli_script(&temp.root);
    let schema_dir = workspace_schema_dir();
    let phases = Arc::new(Mutex::new(Vec::<HookPhase>::new()));

    let config = ClientConfig::new()
        .with_cli_bin(cli)
        .with_schema_dir(schema_dir)
        .with_pre_hook(Arc::new(RecordingPreHook {
            name: "cfg_pre",
            phases: Arc::clone(&phases),
        }))
        .with_post_hook(Arc::new(RecordingPostHook {
            name: "cfg_post",
            phases: Arc::clone(&phases),
        }));
    let client = super::Client::connect(config)
        .await
        .expect("client connect");

    let out = client
        .run(temp.root.to_string_lossy().to_string(), "cfg-hook")
        .await
        .expect("run with cfg hook");
    assert_eq!(out.assistant_text, "cfg-hook");
    assert!(seen_phase(&phases, HookPhase::PreRun));
    assert!(seen_phase(&phases, HookPhase::PostRun));

    client.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn run_profile_hooks_register_and_execute() {
    let temp = TempDir::new("coclai_client_hooks_profile");
    let cli = write_mock_cli_script(&temp.root);
    let schema_dir = workspace_schema_dir();
    let phases = Arc::new(Mutex::new(Vec::<HookPhase>::new()));

    let client = super::Client::connect(
        ClientConfig::new()
            .with_cli_bin(cli)
            .with_schema_dir(schema_dir)
            .with_hooks(RuntimeHookConfig::default()),
    )
    .await
    .expect("client connect");

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
        .run_with_profile(
            temp.root.to_string_lossy().to_string(),
            "profile-hook",
            profile,
        )
        .await
        .expect("run with profile");
    assert_eq!(out.assistant_text, "profile-hook");
    assert!(seen_phase(&phases, HookPhase::PreRun));
    assert!(seen_phase(&phases, HookPhase::PostRun));

    client.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn session_config_hooks_register_and_execute() {
    let temp = TempDir::new("coclai_client_hooks_session");
    let cli = write_mock_cli_script(&temp.root);
    let schema_dir = workspace_schema_dir();
    let phases = Arc::new(Mutex::new(Vec::<HookPhase>::new()));

    let client = super::Client::connect(
        ClientConfig::new()
            .with_cli_bin(cli)
            .with_schema_dir(schema_dir)
            .with_hooks(RuntimeHookConfig::default()),
    )
    .await
    .expect("client connect");

    let session = client
        .start_session(
            SessionConfig::new(temp.root.to_string_lossy().to_string())
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
    let temp = TempDir::new("coclai_client_hooks_no_leak_run");
    let cli = write_mock_cli_script(&temp.root);
    let schema_dir = workspace_schema_dir();
    let phases = Arc::new(Mutex::new(Vec::<HookPhase>::new()));

    let client = super::Client::connect(
        ClientConfig::new()
            .with_cli_bin(cli)
            .with_schema_dir(schema_dir)
            .with_hooks(RuntimeHookConfig::default()),
    )
    .await
    .expect("client connect");

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
        .run_with_profile(temp.root.to_string_lossy().to_string(), "first", profile)
        .await
        .expect("run with profile");
    assert_eq!(first.assistant_text, "first");
    assert_eq!(count_phase(&phases, HookPhase::PreRun), 1);
    assert_eq!(count_phase(&phases, HookPhase::PostRun), 1);

    let second = client
        .run(temp.root.to_string_lossy().to_string(), "second")
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
    let temp = TempDir::new("coclai_client_hooks_no_leak_session");
    let cli = write_mock_cli_script(&temp.root);
    let schema_dir = workspace_schema_dir();
    let phases = Arc::new(Mutex::new(Vec::<HookPhase>::new()));

    let client = super::Client::connect(
        ClientConfig::new()
            .with_cli_bin(cli)
            .with_schema_dir(schema_dir)
            .with_hooks(RuntimeHookConfig::default()),
    )
    .await
    .expect("client connect");

    let session_a = client
        .start_session(
            SessionConfig::new(temp.root.to_string_lossy().to_string())
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
        .start_session(SessionConfig::new(temp.root.to_string_lossy().to_string()))
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
