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
mod tests;
