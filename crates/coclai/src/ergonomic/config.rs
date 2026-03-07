use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crate::plugin::{PostHook, PreHook};
use crate::runtime::{
    ApprovalPolicy, ClientConfig, CompatibilityGuard, PromptAttachment, ReasoningEffort,
    RunProfile, RuntimeHookConfig, SandboxPolicy, SessionConfig,
};

use crate::ergonomic::paths::absolutize_cwd_without_fs_checks;

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
    ///   (non-utf8 absolute paths fall back to caller-provided UTF-8 input without lossy conversion)
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

    /// Replace global runtime hooks (connect-time).
    pub fn with_global_hooks(mut self, hooks: RuntimeHookConfig) -> Self {
        self.client_config = self.client_config.with_hooks(hooks);
        self
    }

    /// Register one global runtime pre hook (connect-time).
    pub fn with_global_pre_hook(mut self, hook: Arc<dyn PreHook>) -> Self {
        self.client_config = self.client_config.with_pre_hook(hook);
        self
    }

    /// Register one global runtime post hook (connect-time).
    pub fn with_global_post_hook(mut self, hook: Arc<dyn PostHook>) -> Self {
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
    pub fn with_run_hooks(mut self, hooks: RuntimeHookConfig) -> Self {
        self.run_profile = self.run_profile.with_hooks(hooks);
        self
    }

    /// Register one run-level pre hook.
    pub fn with_run_pre_hook(mut self, hook: Arc<dyn PreHook>) -> Self {
        self.run_profile = self.run_profile.with_pre_hook(hook);
        self
    }

    /// Register one run-level post hook.
    pub fn with_run_post_hook(mut self, hook: Arc<dyn PostHook>) -> Self {
        self.run_profile = self.run_profile.with_post_hook(hook);
        self
    }

    /// Build session config with the same cwd/profile defaults.
    pub fn to_session_config(&self) -> SessionConfig {
        SessionConfig::from_profile(self.cwd.clone(), self.run_profile.clone())
    }
}
