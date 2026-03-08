use super::super::*;
use super::common::{TestPostHook, TestPreHook};
use crate::runtime::{
    ApprovalPolicy, PromptRunError, PromptRunResult, ReasoningEffort, RuntimeError, SandboxPolicy,
};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

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
    assert!(config.run_profile.hooks.pre_hooks.is_empty());
    assert!(config.run_profile.hooks.post_hooks.is_empty());
}

#[test]
fn workflow_config_builder_supports_expert_overrides() {
    let config = WorkflowConfig::new("/repo")
        .with_model("gpt-5-codex")
        .with_effort(ReasoningEffort::High)
        .with_approval_policy(ApprovalPolicy::OnRequest)
        .attach_path("README.md")
        .with_run_pre_hook(Arc::new(TestPreHook))
        .with_run_post_hook(Arc::new(TestPostHook));

    assert_eq!(config.run_profile.model.as_deref(), Some("gpt-5-codex"));
    assert_eq!(config.run_profile.effort, ReasoningEffort::High);
    assert_eq!(
        config.run_profile.approval_policy,
        ApprovalPolicy::OnRequest
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
        .with_timeout(Duration::from_secs(42))
        .attach_path_with_placeholder("README.md", "readme");
    let session = config.to_session_config();

    assert_eq!(session.cwd, "/repo");
    assert_eq!(session.model.as_deref(), Some("gpt-5-codex"));
    assert_eq!(session.effort, ReasoningEffort::High);
    assert_eq!(session.approval_policy, ApprovalPolicy::OnRequest);
    assert_eq!(session.timeout, Duration::from_secs(42));
    assert_eq!(session.attachments.len(), 1);
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
