use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use coclai_runtime::{
    ApprovalPolicy, Client, ClientConfig, ClientError, CompatibilityGuard, PostHook, PreHook,
    PromptAttachment, PromptRunError, PromptRunResult, ReasoningEffort, RunProfile, RuntimeError,
    RuntimeHookConfig, SandboxPolicy, Session, SessionConfig,
};
use thiserror::Error;

/// One explicit data model for reusable workflow defaults.
/// This keeps simple and advanced paths on a single concrete structure.
#[derive(Clone, Debug, PartialEq)]
pub struct WorkflowConfig {
    pub cwd: String,
    pub client_config: ClientConfig,
    pub run_profile: RunProfile,
}

impl WorkflowConfig {
    /// Create config with safe defaults:
    /// - runtime discovery via `ClientConfig::new()`
    /// - model unset, effort medium, approval never, sandbox read-only
    /// - cwd normalized to absolute path without filesystem existence checks
    pub fn new(cwd: impl Into<String>) -> Self {
        let normalized_cwd = absolutize_cwd_without_fs_checks(&cwd.into());
        Self {
            cwd: normalized_cwd,
            client_config: ClientConfig::new(),
            run_profile: RunProfile::new(),
        }
    }

    /// Replace whole client config.
    pub fn with_client_config(mut self, client_config: ClientConfig) -> Self {
        self.client_config = client_config;
        self
    }

    /// Replace whole run profile.
    pub fn with_run_profile(mut self, run_profile: RunProfile) -> Self {
        self.run_profile = run_profile;
        self
    }

    /// Override codex binary location.
    pub fn with_cli_bin(mut self, cli_bin: impl Into<PathBuf>) -> Self {
        self.client_config = self.client_config.with_cli_bin(cli_bin);
        self
    }

    /// Override schema directory.
    pub fn with_schema_dir(mut self, schema_dir: impl Into<PathBuf>) -> Self {
        self.client_config = self.client_config.with_schema_dir(schema_dir);
        self
    }

    /// Override runtime compatibility policy.
    pub fn with_compatibility_guard(mut self, guard: CompatibilityGuard) -> Self {
        self.client_config = self.client_config.with_compatibility_guard(guard);
        self
    }

    /// Disable compatibility guard.
    pub fn without_compatibility_guard(mut self) -> Self {
        self.client_config = self.client_config.without_compatibility_guard();
        self
    }

    /// Replace global runtime hook config (connect-time).
    pub fn with_runtime_hooks(mut self, hooks: RuntimeHookConfig) -> Self {
        self.client_config = self.client_config.with_hooks(hooks);
        self
    }

    /// Register one global runtime pre hook (connect-time).
    pub fn with_runtime_pre_hook(mut self, hook: Arc<dyn PreHook>) -> Self {
        self.client_config = self.client_config.with_pre_hook(hook);
        self
    }

    /// Register one global runtime post hook (connect-time).
    pub fn with_runtime_post_hook(mut self, hook: Arc<dyn PostHook>) -> Self {
        self.client_config = self.client_config.with_post_hook(hook);
        self
    }

    /// Set explicit model override.
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.run_profile = self.run_profile.with_model(model);
        self
    }

    /// Set explicit reasoning effort.
    pub fn with_effort(mut self, effort: ReasoningEffort) -> Self {
        self.run_profile = self.run_profile.with_effort(effort);
        self
    }

    /// Set approval policy override.
    pub fn with_approval_policy(mut self, approval_policy: ApprovalPolicy) -> Self {
        self.run_profile = self.run_profile.with_approval_policy(approval_policy);
        self
    }

    /// Set sandbox policy override.
    pub fn with_sandbox_policy(mut self, sandbox_policy: SandboxPolicy) -> Self {
        self.run_profile = self.run_profile.with_sandbox_policy(sandbox_policy);
        self
    }

    /// Set prompt timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.run_profile = self.run_profile.with_timeout(timeout);
        self
    }

    /// Add one attachment.
    pub fn with_attachment(mut self, attachment: PromptAttachment) -> Self {
        self.run_profile = self.run_profile.with_attachment(attachment);
        self
    }

    /// Add one `@path` attachment.
    pub fn attach_path(mut self, path: impl Into<String>) -> Self {
        self.run_profile = self.run_profile.attach_path(path);
        self
    }

    /// Add one `@path` attachment with placeholder.
    pub fn attach_path_with_placeholder(
        mut self,
        path: impl Into<String>,
        placeholder: impl Into<String>,
    ) -> Self {
        self.run_profile = self
            .run_profile
            .attach_path_with_placeholder(path, placeholder);
        self
    }

    /// Add one remote image attachment.
    pub fn attach_image_url(mut self, url: impl Into<String>) -> Self {
        self.run_profile = self.run_profile.attach_image_url(url);
        self
    }

    /// Add one local image attachment.
    pub fn attach_local_image(mut self, path: impl Into<String>) -> Self {
        self.run_profile = self.run_profile.attach_local_image(path);
        self
    }

    /// Add one skill attachment.
    pub fn attach_skill(mut self, name: impl Into<String>, path: impl Into<String>) -> Self {
        self.run_profile = self.run_profile.attach_skill(name, path);
        self
    }

    /// Replace run-level hooks.
    pub fn with_hooks(mut self, hooks: RuntimeHookConfig) -> Self {
        self.run_profile = self.run_profile.with_hooks(hooks);
        self
    }

    /// Register one run-level pre hook.
    pub fn with_pre_hook(mut self, hook: Arc<dyn PreHook>) -> Self {
        self.run_profile = self.run_profile.with_pre_hook(hook);
        self
    }

    /// Register one run-level post hook.
    pub fn with_post_hook(mut self, hook: Arc<dyn PostHook>) -> Self {
        self.run_profile = self.run_profile.with_post_hook(hook);
        self
    }

    /// Build session config with the same cwd/profile defaults.
    pub fn to_session_config(&self) -> SessionConfig {
        SessionConfig::from_profile(self.cwd.clone(), self.run_profile.clone())
    }
}

/// One reusable workflow handle:
/// - simple path: `run(prompt)`
/// - expert path: profile/config mutation via `WorkflowConfig`
#[derive(Clone)]
pub struct Workflow {
    client: Client,
    config: WorkflowConfig,
}

impl Workflow {
    /// Connect once with one explicit workflow config.
    pub async fn connect(config: WorkflowConfig) -> Result<Self, ClientError> {
        let client = Client::connect(config.client_config.clone()).await?;
        Ok(Self { client, config })
    }

    /// Connect with defaults for one cwd.
    pub async fn connect_default(cwd: impl Into<String>) -> Result<Self, ClientError> {
        Self::connect(WorkflowConfig::new(cwd)).await
    }

    /// Run one prompt using workflow defaults.
    pub async fn run(&self, prompt: impl Into<String>) -> Result<PromptRunResult, PromptRunError> {
        self.client
            .run_with_profile(
                self.config.cwd.clone(),
                prompt.into(),
                self.config.run_profile.clone(),
            )
            .await
    }

    /// Run one prompt with explicit profile override.
    pub async fn run_with_profile(
        &self,
        prompt: impl Into<String>,
        profile: RunProfile,
    ) -> Result<PromptRunResult, PromptRunError> {
        self.client
            .run_with_profile(self.config.cwd.clone(), prompt.into(), profile)
            .await
    }

    /// Start one session using workflow defaults.
    pub async fn setup_session(&self) -> Result<Session, PromptRunError> {
        self.client
            .start_session(self.config.to_session_config())
            .await
    }

    /// Start one session with explicit profile override.
    pub async fn setup_session_with_profile(
        &self,
        profile: RunProfile,
    ) -> Result<Session, PromptRunError> {
        self.client
            .setup_with_profile(self.config.cwd.clone(), profile)
            .await
    }

    pub fn config(&self) -> &WorkflowConfig {
        &self.config
    }

    pub fn client(&self) -> &Client {
        &self.client
    }

    /// Explicit shutdown to keep lifecycle obvious.
    pub async fn shutdown(self) -> Result<(), RuntimeError> {
        self.client.shutdown().await
    }
}

/// Error model for one-shot convenience calls.
/// Side effects are explicit: run errors can carry shutdown errors.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum QuickRunError {
    #[error("failed to connect codex runtime: {0}")]
    Connect(#[from] ClientError),
    #[error("prompt run failed: {run}; shutdown_error={shutdown:?}")]
    Run {
        run: PromptRunError,
        shutdown: Option<RuntimeError>,
    },
    #[error("runtime shutdown failed after successful run: {0}")]
    Shutdown(#[from] RuntimeError),
}

/// One-shot convenience:
/// connect -> run(default profile) -> shutdown
pub async fn quick_run(
    cwd: impl Into<String>,
    prompt: impl Into<String>,
) -> Result<PromptRunResult, QuickRunError> {
    let client = Client::connect_default().await?;
    let run_result = client.run(cwd.into(), prompt.into()).await;
    let shutdown_result = client.shutdown().await;
    fold_quick_run(run_result, shutdown_result)
}

/// One-shot convenience with explicit profile:
/// connect -> run(profile) -> shutdown
pub async fn quick_run_with_profile(
    cwd: impl Into<String>,
    prompt: impl Into<String>,
    profile: RunProfile,
) -> Result<PromptRunResult, QuickRunError> {
    let client = Client::connect_default().await?;
    let run_result = client
        .run_with_profile(cwd.into(), prompt.into(), profile)
        .await;
    let shutdown_result = client.shutdown().await;
    fold_quick_run(run_result, shutdown_result)
}

fn fold_quick_run(
    run_result: Result<PromptRunResult, PromptRunError>,
    shutdown_result: Result<(), RuntimeError>,
) -> Result<PromptRunResult, QuickRunError> {
    match (run_result, shutdown_result) {
        (Ok(output), Ok(())) => Ok(output),
        (Ok(_), Err(shutdown)) => Err(QuickRunError::Shutdown(shutdown)),
        (Err(run), Ok(())) => Err(QuickRunError::Run {
            run,
            shutdown: None,
        }),
        (Err(run), Err(shutdown)) => Err(QuickRunError::Run {
            run,
            shutdown: Some(shutdown),
        }),
    }
}

fn absolutize_cwd_without_fs_checks(cwd: &str) -> String {
    let path = PathBuf::from(cwd);
    let absolute = if path.is_absolute() {
        path
    } else {
        match std::env::current_dir() {
            Ok(current) => current.join(path),
            Err(_) => path,
        }
    };
    absolute.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use coclai_runtime::{HookAction, HookContext, HookIssue, HookPatch, HookPhase};
    use std::future::Future;
    use std::path::PathBuf;
    use std::pin::Pin;

    struct TestPreHook;
    struct TestPostHook;

    impl PreHook for TestPreHook {
        fn name(&self) -> &'static str {
            "test_pre"
        }

        fn call<'a>(
            &'a self,
            _ctx: &'a HookContext,
        ) -> Pin<Box<dyn Future<Output = Result<HookAction, HookIssue>> + Send + 'a>> {
            Box::pin(async { Ok(HookAction::Mutate(HookPatch::default())) })
        }
    }

    impl PostHook for TestPostHook {
        fn name(&self) -> &'static str {
            "test_post"
        }

        fn call<'a>(
            &'a self,
            _ctx: &'a HookContext,
        ) -> Pin<Box<dyn Future<Output = Result<(), HookIssue>> + Send + 'a>> {
            Box::pin(async { Ok(()) })
        }
    }

    #[test]
    fn workflow_config_defaults_are_safe_and_explicit() {
        let config = WorkflowConfig::new("/tmp/work");
        assert_eq!(config.cwd, "/tmp/work");
        assert_eq!(config.run_profile.effort, ReasoningEffort::Medium);
        assert_eq!(config.run_profile.approval_policy, ApprovalPolicy::Never);
        assert_eq!(
            config.run_profile.sandbox_policy,
            SandboxPolicy::Preset(coclai_runtime::SandboxPreset::ReadOnly)
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
            .with_pre_hook(Arc::new(TestPreHook))
            .with_post_hook(Arc::new(TestPostHook));

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
    fn hook_types_compile_with_current_contract() {
        let pre = TestPreHook;
        let post = TestPostHook;
        assert_eq!(pre.name(), "test_pre");
        assert_eq!(post.name(), "test_post");
        assert!(matches!(HookPhase::PreRun, HookPhase::PreRun));
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
        let relative = format!(
            "coclai_nonexistent_{}_segment",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock before epoch")
                .as_nanos()
        );
        let cfg = WorkflowConfig::new(relative.clone());

        let expected = std::env::current_dir()
            .expect("cwd")
            .join(PathBuf::from(relative));
        assert_eq!(PathBuf::from(cfg.cwd), expected);
    }

    #[test]
    fn workflow_config_new_keeps_absolute_path_stable() {
        let absolute = std::env::temp_dir().join(format!(
            "coclai_abs_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock before epoch")
                .as_nanos()
        ));
        let cfg = WorkflowConfig::new(absolute.to_string_lossy().to_string());
        assert_eq!(PathBuf::from(cfg.cwd), absolute);
    }
}
