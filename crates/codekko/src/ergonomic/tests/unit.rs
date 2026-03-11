use super::super::*;
use super::common::{TestPostHook, TestPreHook};
use crate::runtime::{
    ApprovalPolicy, InitializeCapabilities, PromptRunError, PromptRunResult, ReasoningEffort,
    RuntimeError, SandboxPolicy,
};
use serde_json::json;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

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
        sys.stdout.write(json.dumps({"id": rpc_id, "result": {"thread": {"id": "thr_workflow"}}}) + "\n")
        sys.stdout.flush()
        continue

    if method == "turn/start":
        thread_id = params.get("threadId") or "thr_workflow"
        turn_id = "turn_workflow"
        text = "ok"
        if params.get("outputSchema") is not None:
            text = json.dumps(params.get("outputSchema"), sort_keys=True)

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

#[test]
fn workflow_config_defaults_are_safe_and_explicit() {
    let config = WorkflowConfig::new("/tmp/work");
    assert_eq!(config.cwd, "/tmp/work");
    assert_eq!(config.run_profile.effort, ReasoningEffort::Medium);
    assert_eq!(config.run_profile.approval_policy, ApprovalPolicy::Never);
    assert_eq!(
        config.run_profile.sandbox_policy,
        SandboxPolicy::Preset(crate::runtime::SandboxPreset::ReadOnly)
    );
    assert!(config.run_profile.attachments.is_empty());
    assert_eq!(config.run_profile.output_schema, None);
    assert!(config.run_profile.hooks.pre_hooks.is_empty());
    assert!(config.run_profile.hooks.post_hooks.is_empty());
}

#[test]
fn workflow_config_builder_supports_expert_overrides() {
    let config = WorkflowConfig::new("/repo")
        .with_model("gpt-5-codex")
        .with_effort(ReasoningEffort::High)
        .with_approval_policy(ApprovalPolicy::OnRequest)
        .with_output_schema(json!({"type":"object","properties":{"result":{"type":"string"}}}))
        .attach_path("README.md")
        .with_run_pre_hook(Arc::new(TestPreHook))
        .with_run_post_hook(Arc::new(TestPostHook));

    assert_eq!(config.run_profile.model.as_deref(), Some("gpt-5-codex"));
    assert_eq!(config.run_profile.effort, ReasoningEffort::High);
    assert_eq!(
        config.run_profile.approval_policy,
        ApprovalPolicy::OnRequest
    );
    assert_eq!(
        config.run_profile.output_schema,
        Some(json!({"type":"object","properties":{"result":{"type":"string"}}}))
    );
    assert_eq!(config.run_profile.attachments.len(), 1);
    assert_eq!(config.run_profile.hooks.pre_hooks.len(), 1);
    assert_eq!(config.run_profile.hooks.post_hooks.len(), 1);
}

#[test]
fn to_session_config_projects_profile_without_loss() {
    let config = WorkflowConfig::new("/repo")
        .with_model("gpt-5-codex")
        .with_effort(ReasoningEffort::High)
        .with_approval_policy(ApprovalPolicy::OnRequest)
        .with_output_schema(json!({"type":"object","required":["value"]}))
        .with_timeout(Duration::from_secs(42))
        .attach_path_with_placeholder("README.md", "readme");
    let session = config.to_session_config();

    assert_eq!(session.cwd, "/repo");
    assert_eq!(session.model.as_deref(), Some("gpt-5-codex"));
    assert_eq!(session.effort, ReasoningEffort::High);
    assert_eq!(session.approval_policy, ApprovalPolicy::OnRequest);
    assert_eq!(
        session.output_schema,
        Some(json!({"type":"object","required":["value"]}))
    );
    assert_eq!(session.timeout, Duration::from_secs(42));
    assert_eq!(session.attachments.len(), 1);
}

#[test]
fn workflow_config_can_enable_experimental_api() {
    let config = WorkflowConfig::new("/repo").enable_experimental_api();
    assert_eq!(
        config.client_config.initialize_capabilities,
        InitializeCapabilities::new().enable_experimental_api()
    );
}

#[tokio::test(flavor = "current_thread")]
async fn workflow_run_propagates_output_schema_to_turn_start() {
    let temp = TempDir::new("ergonomic_workflow_output_schema");
    let cli = write_mock_cli_script(&temp.root);
    let schema = json!({
        "type": "object",
        "required": ["value"],
        "properties": {
            "value": {"type": "string"}
        }
    });
    let workflow = Workflow::connect(
        WorkflowConfig::new(
            temp.root
                .to_str()
                .expect("temp dir path must be utf-8 in this test"),
        )
        .with_cli_bin(cli)
        .with_output_schema(schema.clone()),
    )
    .await
    .expect("connect workflow");

    let out = workflow.run("schema-workflow").await.expect("run");
    let echoed: serde_json::Value =
        serde_json::from_str(&out.assistant_text).expect("assistant text must echo schema");
    assert_eq!(echoed, schema);

    workflow.shutdown().await.expect("shutdown");
}

#[test]
fn fold_quick_run_returns_output_when_run_and_shutdown_succeed() {
    let out = PromptRunResult {
        thread_id: "thread-1".to_owned(),
        turn_id: "turn-1".to_owned(),
        assistant_text: "ok".to_owned(),
    };
    let result = fold_quick_run(Ok(out.clone()), Ok(()));
    assert_eq!(result, Ok(out));
}

#[test]
fn fold_quick_run_returns_shutdown_error_after_successful_run() {
    let out = PromptRunResult {
        thread_id: "thread-1".to_owned(),
        turn_id: "turn-1".to_owned(),
        assistant_text: "ok".to_owned(),
    };
    let result = fold_quick_run(Ok(out), Err(RuntimeError::Internal("shutdown".to_owned())));
    assert_eq!(
        result,
        Err(QuickRunError::Shutdown(RuntimeError::Internal(
            "shutdown".to_owned()
        )))
    );
}

#[test]
fn fold_quick_run_carries_shutdown_error_when_run_fails() {
    let result = fold_quick_run(
        Err(PromptRunError::TurnFailed),
        Err(RuntimeError::Internal("shutdown".to_owned())),
    );
    assert_eq!(
        result,
        Err(QuickRunError::Run {
            run: PromptRunError::TurnFailed,
            shutdown: Some(RuntimeError::Internal("shutdown".to_owned())),
        })
    );
}

#[test]
fn workflow_config_new_makes_relative_path_absolute_without_fs_checks() {
    let relative = "runtime_relative_path_without_fs_check";
    let cfg = WorkflowConfig::new(relative);

    let expected = std::env::current_dir()
        .expect("cwd")
        .join(PathBuf::from(relative));
    assert_eq!(PathBuf::from(cfg.cwd), expected);
}

#[test]
fn workflow_config_new_keeps_absolute_path_stable() {
    let absolute = std::env::temp_dir().join("runtime_abs_path_stable");
    let absolute_utf8 = absolute
        .to_str()
        .expect("temp dir path must be utf-8 in this test");
    let cfg = WorkflowConfig::new(absolute_utf8.to_owned());
    assert_eq!(PathBuf::from(cfg.cwd), absolute);
}
