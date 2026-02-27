use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use coclai_plugin_core::{PostHook, PreHook};

use crate::api::{
    ApprovalPolicy, PromptAttachment, PromptRunParams, ReasoningEffort, SandboxPolicy,
    SandboxPreset, ThreadStartParams, DEFAULT_REASONING_EFFORT,
};
use crate::hooks::RuntimeHookConfig;

#[derive(Clone, Debug, PartialEq)]
pub struct RunProfile {
    pub model: Option<String>,
    pub effort: ReasoningEffort,
    pub approval_policy: ApprovalPolicy,
    pub sandbox_policy: SandboxPolicy,
    /// Explicit opt-in gate for privileged sandbox usage (SEC-004).
    pub privileged_escalation_approved: bool,
    pub attachments: Vec<PromptAttachment>,
    pub timeout: Duration,
    pub hooks: RuntimeHookConfig,
}

impl Default for RunProfile {
    fn default() -> Self {
        Self {
            model: None,
            effort: DEFAULT_REASONING_EFFORT,
            approval_policy: ApprovalPolicy::Never,
            sandbox_policy: SandboxPolicy::Preset(SandboxPreset::ReadOnly),
            privileged_escalation_approved: false,
            attachments: Vec::new(),
            timeout: Duration::from_secs(120),
            hooks: RuntimeHookConfig::default(),
        }
    }
}

impl RunProfile {
    /// Create reusable run/session profile with safe defaults.
    /// Allocation: none. Complexity: O(1).
    pub fn new() -> Self {
        Self::default()
    }

    /// Set explicit model override.
    /// Allocation: one String. Complexity: O(model length).
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// Set explicit reasoning effort.
    /// Allocation: none. Complexity: O(1).
    pub fn with_effort(mut self, effort: ReasoningEffort) -> Self {
        self.effort = effort;
        self
    }

    /// Set approval policy override.
    /// Allocation: none. Complexity: O(1).
    pub fn with_approval_policy(mut self, approval_policy: ApprovalPolicy) -> Self {
        self.approval_policy = approval_policy;
        self
    }

    /// Set sandbox policy override.
    /// Allocation: depends on payload move/clone at callsite. Complexity: O(1).
    pub fn with_sandbox_policy(mut self, sandbox_policy: SandboxPolicy) -> Self {
        self.sandbox_policy = sandbox_policy;
        self
    }

    /// Explicitly approve privileged sandbox escalation for runs using this profile.
    pub fn allow_privileged_escalation(mut self) -> Self {
        self.privileged_escalation_approved = true;
        self
    }

    /// Set timeout.
    /// Allocation: none. Complexity: O(1).
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Add one generic attachment.
    /// Allocation: amortized O(1) push. Complexity: O(1).
    pub fn with_attachment(mut self, attachment: PromptAttachment) -> Self {
        self.attachments.push(attachment);
        self
    }

    /// Add one `@path` attachment.
    /// Allocation: one String. Complexity: O(path length).
    pub fn attach_path(self, path: impl Into<String>) -> Self {
        self.with_attachment(PromptAttachment::AtPath {
            path: path.into(),
            placeholder: None,
        })
    }

    /// Add one `@path` attachment with placeholder.
    /// Allocation: two Strings. Complexity: O(path + placeholder length).
    pub fn attach_path_with_placeholder(
        self,
        path: impl Into<String>,
        placeholder: impl Into<String>,
    ) -> Self {
        self.with_attachment(PromptAttachment::AtPath {
            path: path.into(),
            placeholder: Some(placeholder.into()),
        })
    }

    /// Add one remote image attachment.
    /// Allocation: one String. Complexity: O(url length).
    pub fn attach_image_url(self, url: impl Into<String>) -> Self {
        self.with_attachment(PromptAttachment::ImageUrl { url: url.into() })
    }

    /// Add one local image attachment.
    /// Allocation: one String. Complexity: O(path length).
    pub fn attach_local_image(self, path: impl Into<String>) -> Self {
        self.with_attachment(PromptAttachment::LocalImage { path: path.into() })
    }

    /// Add one skill attachment.
    /// Allocation: two Strings. Complexity: O(name + path length).
    pub fn attach_skill(self, name: impl Into<String>, path: impl Into<String>) -> Self {
        self.with_attachment(PromptAttachment::Skill {
            name: name.into(),
            path: path.into(),
        })
    }

    /// Replace hook configuration for runs using this profile.
    /// Allocation: O(h), h = hook count. Complexity: O(1) move.
    pub fn with_hooks(mut self, hooks: RuntimeHookConfig) -> Self {
        self.hooks = hooks;
        self
    }

    /// Register one pre hook for runs using this profile.
    /// Allocation: amortized O(1) push. Complexity: O(1).
    pub fn with_pre_hook(mut self, hook: Arc<dyn PreHook>) -> Self {
        self.hooks.pre_hooks.push(hook);
        self
    }

    /// Register one post hook for runs using this profile.
    /// Allocation: amortized O(1) push. Complexity: O(1).
    pub fn with_post_hook(mut self, hook: Arc<dyn PostHook>) -> Self {
        self.hooks.post_hooks.push(hook);
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SessionConfig {
    pub cwd: String,
    pub model: Option<String>,
    pub effort: ReasoningEffort,
    pub approval_policy: ApprovalPolicy,
    pub sandbox_policy: SandboxPolicy,
    /// Explicit opt-in gate for privileged sandbox usage (SEC-004).
    pub privileged_escalation_approved: bool,
    pub attachments: Vec<PromptAttachment>,
    pub timeout: Duration,
    pub hooks: RuntimeHookConfig,
}

impl SessionConfig {
    /// Create session config with safe defaults.
    /// Allocation: one String for cwd. Complexity: O(cwd length).
    pub fn new(cwd: impl Into<String>) -> Self {
        Self::from_profile(cwd, RunProfile::default())
    }

    /// Create session config from one reusable run profile.
    /// Allocation: one String for cwd + profile field moves. Complexity: O(cwd length).
    pub fn from_profile(cwd: impl Into<String>, profile: RunProfile) -> Self {
        Self {
            cwd: cwd.into(),
            model: profile.model,
            effort: profile.effort,
            approval_policy: profile.approval_policy,
            sandbox_policy: profile.sandbox_policy,
            privileged_escalation_approved: profile.privileged_escalation_approved,
            attachments: profile.attachments,
            timeout: profile.timeout,
            hooks: profile.hooks,
        }
    }

    /// Materialize profile view of this session defaults.
    /// Allocation: clones Strings/attachments. Complexity: O(n), n = attachment count + string sizes.
    pub fn profile(&self) -> RunProfile {
        RunProfile {
            model: self.model.clone(),
            effort: self.effort,
            approval_policy: self.approval_policy,
            sandbox_policy: self.sandbox_policy.clone(),
            privileged_escalation_approved: self.privileged_escalation_approved,
            attachments: self.attachments.clone(),
            timeout: self.timeout,
            hooks: self.hooks.clone(),
        }
    }

    /// Set explicit model override.
    /// Allocation: one String. Complexity: O(model length).
    pub fn with_model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// Set explicit reasoning effort.
    /// Allocation: none. Complexity: O(1).
    pub fn with_effort(mut self, effort: ReasoningEffort) -> Self {
        self.effort = effort;
        self
    }

    /// Set approval policy override.
    /// Allocation: none. Complexity: O(1).
    pub fn with_approval_policy(mut self, approval_policy: ApprovalPolicy) -> Self {
        self.approval_policy = approval_policy;
        self
    }

    /// Set sandbox policy override.
    /// Allocation: depends on payload move/clone at callsite. Complexity: O(1).
    pub fn with_sandbox_policy(mut self, sandbox_policy: SandboxPolicy) -> Self {
        self.sandbox_policy = sandbox_policy;
        self
    }

    /// Explicitly approve privileged sandbox escalation for turns in this session.
    pub fn allow_privileged_escalation(mut self) -> Self {
        self.privileged_escalation_approved = true;
        self
    }

    /// Set timeout for each session turn.
    /// Allocation: none. Complexity: O(1).
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Add one generic attachment that is reused for each turn.
    /// Allocation: amortized O(1) push. Complexity: O(1).
    pub fn with_attachment(mut self, attachment: PromptAttachment) -> Self {
        self.attachments.push(attachment);
        self
    }

    /// Add one `@path` attachment reused for each turn.
    /// Allocation: one String. Complexity: O(path length).
    pub fn attach_path(self, path: impl Into<String>) -> Self {
        self.with_attachment(PromptAttachment::AtPath {
            path: path.into(),
            placeholder: None,
        })
    }

    /// Add one `@path` attachment with placeholder reused for each turn.
    /// Allocation: two Strings. Complexity: O(path + placeholder length).
    pub fn attach_path_with_placeholder(
        self,
        path: impl Into<String>,
        placeholder: impl Into<String>,
    ) -> Self {
        self.with_attachment(PromptAttachment::AtPath {
            path: path.into(),
            placeholder: Some(placeholder.into()),
        })
    }

    /// Add one remote image attachment reused for each turn.
    /// Allocation: one String. Complexity: O(url length).
    pub fn attach_image_url(self, url: impl Into<String>) -> Self {
        self.with_attachment(PromptAttachment::ImageUrl { url: url.into() })
    }

    /// Add one local image attachment reused for each turn.
    /// Allocation: one String. Complexity: O(path length).
    pub fn attach_local_image(self, path: impl Into<String>) -> Self {
        self.with_attachment(PromptAttachment::LocalImage { path: path.into() })
    }

    /// Add one skill attachment reused for each turn.
    /// Allocation: two Strings. Complexity: O(name + path length).
    pub fn attach_skill(self, name: impl Into<String>, path: impl Into<String>) -> Self {
        self.with_attachment(PromptAttachment::Skill {
            name: name.into(),
            path: path.into(),
        })
    }

    /// Replace hook configuration for this session defaults.
    /// Allocation: O(h), h = hook count. Complexity: O(1) move.
    pub fn with_hooks(mut self, hooks: RuntimeHookConfig) -> Self {
        self.hooks = hooks;
        self
    }

    /// Register one pre hook for this session defaults.
    /// Allocation: amortized O(1) push. Complexity: O(1).
    pub fn with_pre_hook(mut self, hook: Arc<dyn PreHook>) -> Self {
        self.hooks.pre_hooks.push(hook);
        self
    }

    /// Register one post hook for this session defaults.
    /// Allocation: amortized O(1) push. Complexity: O(1).
    pub fn with_post_hook(mut self, hook: Arc<dyn PostHook>) -> Self {
        self.hooks.post_hooks.push(hook);
        self
    }
}

/// Pure transform from reusable session config + prompt into one prompt-run request.
/// Allocation: clones config-owned Strings/vectors + one prompt String. Complexity: O(n), n = attachment count + field sizes.
pub(super) fn session_prompt_params(
    config: &SessionConfig,
    prompt: impl Into<String>,
) -> PromptRunParams {
    PromptRunParams {
        cwd: config.cwd.clone(),
        prompt: prompt.into(),
        model: config.model.clone(),
        effort: Some(config.effort),
        approval_policy: config.approval_policy,
        sandbox_policy: config.sandbox_policy.clone(),
        privileged_escalation_approved: config.privileged_escalation_approved,
        attachments: config.attachments.clone(),
        timeout: config.timeout,
    }
}

/// Pure transform from reusable profile + turn input into one prompt-run request.
/// Allocation: moves profile-owned Strings/vectors + one prompt String. Complexity: O(n), n = attachment count + field sizes.
pub(super) fn profile_to_prompt_params(
    cwd: String,
    prompt: impl Into<String>,
    profile: RunProfile,
) -> PromptRunParams {
    PromptRunParams {
        cwd,
        prompt: prompt.into(),
        model: profile.model,
        effort: Some(profile.effort),
        approval_policy: profile.approval_policy,
        sandbox_policy: profile.sandbox_policy,
        privileged_escalation_approved: profile.privileged_escalation_approved,
        attachments: profile.attachments,
        timeout: profile.timeout,
    }
}

/// Pure transform from session defaults into thread-start/resume overrides.
/// Allocation: clones Strings/policy payloads from config. Complexity: O(n), n = field sizes.
pub(super) fn session_thread_start_params(config: &SessionConfig) -> ThreadStartParams {
    ThreadStartParams {
        model: config.model.clone(),
        cwd: Some(config.cwd.clone()),
        approval_policy: Some(config.approval_policy),
        sandbox_policy: Some(config.sandbox_policy.clone()),
        privileged_escalation_approved: config.privileged_escalation_approved,
    }
}

/// Merge session-default hooks with per-call profile hooks.
/// Ordering is overlay-first to let per-call hooks win on duplicate names.
pub(super) fn merge_hook_configs(
    defaults: &RuntimeHookConfig,
    overlay: &RuntimeHookConfig,
) -> RuntimeHookConfig {
    if defaults.is_empty() {
        return overlay.clone();
    }
    if overlay.is_empty() {
        return defaults.clone();
    }
    RuntimeHookConfig {
        pre_hooks: merge_pre_hooks(&defaults.pre_hooks, &overlay.pre_hooks),
        post_hooks: merge_post_hooks(&defaults.post_hooks, &overlay.post_hooks),
    }
}

fn merge_pre_hooks(
    defaults: &[Arc<dyn PreHook>],
    overlay: &[Arc<dyn PreHook>],
) -> Vec<Arc<dyn PreHook>> {
    let mut merged = Vec::with_capacity(defaults.len() + overlay.len());
    let mut names: HashSet<&'static str> = HashSet::with_capacity(defaults.len() + overlay.len());
    for hook in overlay {
        if names.insert(hook.name()) {
            merged.push(Arc::clone(hook));
        }
    }
    for hook in defaults {
        if names.insert(hook.name()) {
            merged.push(Arc::clone(hook));
        }
    }
    merged
}

fn merge_post_hooks(
    defaults: &[Arc<dyn PostHook>],
    overlay: &[Arc<dyn PostHook>],
) -> Vec<Arc<dyn PostHook>> {
    let mut merged = Vec::with_capacity(defaults.len() + overlay.len());
    let mut names: HashSet<&'static str> = HashSet::with_capacity(defaults.len() + overlay.len());
    for hook in overlay {
        if names.insert(hook.name()) {
            merged.push(Arc::clone(hook));
        }
    }
    for hook in defaults {
        if names.insert(hook.name()) {
            merged.push(Arc::clone(hook));
        }
    }
    merged
}
