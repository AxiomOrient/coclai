use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use thiserror::Error;

use crate::api::{
    ApprovalPolicy, PromptAttachment, PromptRunError, PromptRunParams, PromptRunResult,
    ReasoningEffort, SandboxPolicy, SandboxPreset, ThreadStartParams, DEFAULT_REASONING_EFFORT,
};
use crate::errors::{RpcError, RuntimeError};
use crate::hooks::RuntimeHookConfig;
use crate::runtime::{Runtime, RuntimeConfig, SchemaGuardConfig};
use crate::transport::StdioProcessSpec;
use coclai_plugin_core::{PostHook, PreHook};

pub const DEFAULT_SCHEMA_RELATIVE_DIR: &str = "SCHEMAS/app-server/active";
pub const SCHEMA_DIR_ENV: &str = "APP_SERVER_SCHEMA_DIR";
const PACKAGE_SCHEMA_RELATIVE_FROM_CRATE: &str = "../../SCHEMAS/app-server/active";
const DEFAULT_MIN_CODEX_VERSION: SemVerTriplet = SemVerTriplet {
    major: 0,
    minor: 104,
    patch: 0,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SemVerTriplet {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl SemVerTriplet {
    pub const fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }
}

impl std::fmt::Display for SemVerTriplet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompatibilityGuard {
    pub require_initialize_user_agent: bool,
    pub min_codex_version: Option<SemVerTriplet>,
}

impl Default for CompatibilityGuard {
    fn default() -> Self {
        Self {
            require_initialize_user_agent: true,
            min_codex_version: Some(DEFAULT_MIN_CODEX_VERSION),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientConfig {
    pub cli_bin: PathBuf,
    pub schema_dir: Option<PathBuf>,
    pub compatibility_guard: CompatibilityGuard,
    pub hooks: RuntimeHookConfig,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            cli_bin: PathBuf::from("codex"),
            schema_dir: None,
            compatibility_guard: CompatibilityGuard::default(),
            hooks: RuntimeHookConfig::default(),
        }
    }
}

impl ClientConfig {
    /// Create config with default binary/schema discovery.
    /// Allocation: none. Complexity: O(1).
    pub fn new() -> Self {
        Self::default()
    }

    /// Override CLI executable path.
    /// Allocation: one PathBuf move/clone from input. Complexity: O(path length).
    pub fn with_cli_bin(mut self, cli_bin: impl Into<PathBuf>) -> Self {
        self.cli_bin = cli_bin.into();
        self
    }

    /// Override schema directory explicitly.
    /// Allocation: one PathBuf move/clone from input. Complexity: O(path length).
    pub fn with_schema_dir(mut self, schema_dir: impl Into<PathBuf>) -> Self {
        self.schema_dir = Some(schema_dir.into());
        self
    }

    /// Override runtime compatibility guard policy.
    /// Allocation: none. Complexity: O(1).
    pub fn with_compatibility_guard(mut self, guard: CompatibilityGuard) -> Self {
        self.compatibility_guard = guard;
        self
    }

    /// Disable compatibility guard checks at connect time.
    /// Allocation: none. Complexity: O(1).
    pub fn without_compatibility_guard(mut self) -> Self {
        self.compatibility_guard = CompatibilityGuard {
            require_initialize_user_agent: false,
            min_codex_version: None,
        };
        self
    }

    /// Replace runtime hook configuration.
    /// Allocation: O(h), h = hook count. Complexity: O(1) move.
    pub fn with_hooks(mut self, hooks: RuntimeHookConfig) -> Self {
        self.hooks = hooks;
        self
    }

    /// Register one pre hook on client runtime config.
    /// Allocation: amortized O(1) push. Complexity: O(1).
    pub fn with_pre_hook(mut self, hook: Arc<dyn PreHook>) -> Self {
        self.hooks.pre_hooks.push(hook);
        self
    }

    /// Register one post hook on client runtime config.
    /// Allocation: amortized O(1) push. Complexity: O(1).
    pub fn with_post_hook(mut self, hook: Arc<dyn PostHook>) -> Self {
        self.hooks.post_hooks.push(hook);
        self
    }

    /// Resolve schema directory from explicit config, env, then cwd-relative default.
    /// Side effects: reads process env and current dir.
    /// Allocation: one PathBuf. Complexity: O(path length).
    pub fn resolve_schema_dir(&self) -> Result<PathBuf, ClientError> {
        if let Some(path) = self.schema_dir.as_ref() {
            return validate_schema_dir(path.clone());
        }

        let env_value = std::env::var(SCHEMA_DIR_ENV).ok();
        if let Some(candidate) = env_value
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return validate_schema_dir(PathBuf::from(candidate));
        }

        let cwd =
            std::env::current_dir().map_err(|err| ClientError::CurrentDir(err.to_string()))?;
        let package_default = package_schema_dir();
        resolve_default_schema_dir(&cwd, &package_default)
    }
}

#[derive(Clone)]
pub struct Client {
    runtime: Runtime,
    config: ClientConfig,
    schema_dir: PathBuf,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RunProfile {
    pub model: Option<String>,
    pub effort: ReasoningEffort,
    pub approval_policy: ApprovalPolicy,
    pub sandbox_policy: SandboxPolicy,
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

#[derive(Clone)]
pub struct Session {
    runtime: Runtime,
    pub thread_id: String,
    pub config: SessionConfig,
    closed: Arc<AtomicBool>,
}

impl Session {
    /// Returns true when this local session handle is closed.
    /// Allocation: none. Complexity: O(1).
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    /// Continue this session with one prompt.
    /// Side effects: sends thread/resume + turn/start RPC calls to app-server.
    /// Allocation: PromptRunParams clone payloads (cwd/model/sandbox/attachments). Complexity: O(n), n = attachment count + prompt length.
    pub async fn ask(&self, prompt: impl Into<String>) -> Result<PromptRunResult, PromptRunError> {
        ensure_session_open_for_prompt(self.is_closed())?;
        self.runtime
            .run_prompt_in_thread_with_hooks(
                &self.thread_id,
                session_prompt_params(&self.config, prompt),
                Some(&self.config.hooks),
            )
            .await
    }

    /// Continue this session with one prompt while overriding selected turn options.
    /// Side effects: sends thread/resume + turn/start RPC calls to app-server.
    /// Allocation: depends on caller-provided params. Complexity: O(1) wrapper.
    pub async fn ask_with(
        &self,
        params: PromptRunParams,
    ) -> Result<PromptRunResult, PromptRunError> {
        ensure_session_open_for_prompt(self.is_closed())?;
        self.runtime
            .run_prompt_in_thread_with_hooks(&self.thread_id, params, Some(&self.config.hooks))
            .await
    }

    /// Continue this session with one prompt using one explicit profile override.
    /// Side effects: sends thread/resume + turn/start RPC calls to app-server.
    /// Allocation: moves profile-owned Strings/vectors + one prompt String. Complexity: O(n), n = attachment count + field sizes.
    pub async fn ask_with_profile(
        &self,
        prompt: impl Into<String>,
        profile: RunProfile,
    ) -> Result<PromptRunResult, PromptRunError> {
        ensure_session_open_for_prompt(self.is_closed())?;
        let merged_hooks = merge_hook_configs(&self.config.hooks, &profile.hooks);
        self.runtime
            .run_prompt_in_thread_with_hooks(
                &self.thread_id,
                profile_to_prompt_params(self.config.cwd.clone(), prompt, profile),
                Some(&merged_hooks),
            )
            .await
    }

    /// Return current session default profile snapshot.
    /// Allocation: clones Strings/attachments. Complexity: O(n), n = attachment count + string sizes.
    pub fn profile(&self) -> RunProfile {
        self.config.profile()
    }

    /// Interrupt one in-flight turn in this session.
    /// Side effects: sends turn/interrupt RPC call to app-server.
    /// Allocation: one small JSON payload in runtime layer. Complexity: O(1).
    pub async fn interrupt_turn(&self, turn_id: &str) -> Result<(), RpcError> {
        ensure_session_open_for_rpc(self.is_closed())?;
        self.runtime.turn_interrupt(&self.thread_id, turn_id).await
    }

    /// Archive this session on server side.
    /// Side effects: sends thread/archive RPC call to app-server.
    /// Allocation: one small JSON payload in runtime layer. Complexity: O(1).
    pub async fn close(&self) -> Result<(), RpcError> {
        if self.closed.load(Ordering::Acquire) {
            return Ok(());
        }
        self.closed.store(true, Ordering::Release);
        if let Err(err) = self.runtime.thread_archive(&self.thread_id).await {
            self.closed.store(false, Ordering::Release);
            return Err(err);
        }
        Ok(())
    }
}

impl Client {
    /// Connect using default config (default CLI + schema discovery).
    /// Side effects: spawns `<cli_bin> app-server`.
    /// Allocation: runtime buffers + internal channels.
    pub async fn connect_default() -> Result<Self, ClientError> {
        Self::connect(ClientConfig::new()).await
    }

    /// Connect using explicit client config.
    /// Side effects: validates schema dir, spawns `<cli_bin> app-server`,
    /// and validates initialize compatibility guard.
    /// Allocation: runtime buffers + internal channels.
    pub async fn connect(config: ClientConfig) -> Result<Self, ClientError> {
        let schema_dir = config.resolve_schema_dir()?;

        let mut process = StdioProcessSpec::new(config.cli_bin.clone());
        process.args = vec!["app-server".to_owned()];

        let runtime = Runtime::spawn_local(
            RuntimeConfig::new(
                process,
                SchemaGuardConfig {
                    active_schema_dir: schema_dir.clone(),
                },
            )
            .with_hooks(config.hooks.clone()),
        )
        .await?;
        if let Err(err) = validate_runtime_compatibility(&runtime, &config.compatibility_guard) {
            let _ = runtime.shutdown().await;
            return Err(err);
        }

        Ok(Self {
            runtime,
            config,
            schema_dir,
        })
    }

    /// Run one prompt using default policies (approval=never, sandbox=read-only).
    /// Side effects: sends thread/turn RPC calls to app-server.
    pub async fn run(
        &self,
        cwd: impl Into<String>,
        prompt: impl Into<String>,
    ) -> Result<PromptRunResult, PromptRunError> {
        self.runtime.run_prompt_simple(cwd, prompt).await
    }

    /// Run one prompt with explicit model/policy/attachment options.
    /// Side effects: sends thread/turn RPC calls to app-server.
    pub async fn run_with(
        &self,
        params: PromptRunParams,
    ) -> Result<PromptRunResult, PromptRunError> {
        self.runtime.run_prompt(params).await
    }

    /// Run one prompt with one reusable profile (model/effort/policy/attachments/timeout).
    /// Side effects: sends thread/turn RPC calls to app-server.
    /// Allocation: moves profile-owned Strings/vectors + one prompt String. Complexity: O(n), n = attachment count + field sizes.
    pub async fn run_with_profile(
        &self,
        cwd: impl Into<String>,
        prompt: impl Into<String>,
        profile: RunProfile,
    ) -> Result<PromptRunResult, PromptRunError> {
        let scoped_hooks = profile.hooks.clone();
        self.runtime
            .run_prompt_with_hooks(
                profile_to_prompt_params(cwd.into(), prompt, profile),
                Some(&scoped_hooks),
            )
            .await
    }

    /// Start one default session quickly (safe defaults).
    /// Side effects: sends thread/start RPC call to app-server.
    /// Allocation: one cwd String. Complexity: O(cwd length).
    pub async fn setup(&self, cwd: impl Into<String>) -> Result<Session, PromptRunError> {
        self.start_session(SessionConfig::new(cwd)).await
    }

    /// Start one session from explicit reusable profile.
    /// Side effects: sends thread/start RPC call to app-server.
    /// Allocation: one cwd String + profile field moves. Complexity: O(n), n = attachment count + field sizes.
    pub async fn setup_with_profile(
        &self,
        cwd: impl Into<String>,
        profile: RunProfile,
    ) -> Result<Session, PromptRunError> {
        self.start_session(SessionConfig::from_profile(cwd, profile))
            .await
    }

    /// Start a prepared session and return a reusable handle.
    /// Side effects: sends thread/start RPC call to app-server.
    /// Allocation: clones model/cwd/sandbox into thread-start payload. Complexity: O(n), n = total field sizes.
    pub async fn start_session(&self, config: SessionConfig) -> Result<Session, PromptRunError> {
        let thread = self
            .runtime
            .thread_start_with_hooks(session_thread_start_params(&config), Some(&config.hooks))
            .await?;

        Ok(Session {
            runtime: self.runtime.clone(),
            thread_id: thread.thread_id,
            config,
            closed: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Resume an existing session id with prepared defaults.
    /// Side effects: sends thread/resume RPC call to app-server.
    /// Allocation: clones model/cwd/sandbox into thread-resume payload. Complexity: O(n), n = total field sizes.
    pub async fn resume_session(
        &self,
        thread_id: &str,
        config: SessionConfig,
    ) -> Result<Session, PromptRunError> {
        let thread = self
            .runtime
            .thread_resume_with_hooks(
                thread_id,
                session_thread_start_params(&config),
                Some(&config.hooks),
            )
            .await?;

        Ok(Session {
            runtime: self.runtime.clone(),
            thread_id: thread.thread_id,
            config,
            closed: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Continue an existing thread with one more prompt using default policies.
    /// Side effects: sends thread/resume + turn/start RPC calls to app-server.
    pub async fn continue_session(
        &self,
        thread_id: &str,
        cwd: impl Into<String>,
        prompt: impl Into<String>,
    ) -> Result<PromptRunResult, PromptRunError> {
        self.runtime
            .run_prompt_in_thread(thread_id, PromptRunParams::new(cwd, prompt))
            .await
    }

    /// Continue an existing thread with explicit model/policy/attachment options.
    /// Side effects: sends thread/resume + turn/start RPC calls to app-server.
    pub async fn continue_session_with(
        &self,
        thread_id: &str,
        params: PromptRunParams,
    ) -> Result<PromptRunResult, PromptRunError> {
        self.runtime.run_prompt_in_thread(thread_id, params).await
    }

    /// Continue an existing thread with one reusable profile.
    /// Side effects: sends thread/resume + turn/start RPC calls to app-server.
    /// Allocation: moves profile-owned Strings/vectors + one prompt String. Complexity: O(n), n = attachment count + field sizes.
    pub async fn continue_session_with_profile(
        &self,
        thread_id: &str,
        cwd: impl Into<String>,
        prompt: impl Into<String>,
        profile: RunProfile,
    ) -> Result<PromptRunResult, PromptRunError> {
        let scoped_hooks = profile.hooks.clone();
        self.runtime
            .run_prompt_in_thread_with_hooks(
                thread_id,
                profile_to_prompt_params(cwd.into(), prompt, profile),
                Some(&scoped_hooks),
            )
            .await
    }

    /// Interrupt one in-flight turn by session(thread) id and turn id.
    /// Side effects: sends turn/interrupt RPC call to app-server.
    pub async fn interrupt_session_turn(
        &self,
        thread_id: &str,
        turn_id: &str,
    ) -> Result<(), RpcError> {
        self.runtime.turn_interrupt(thread_id, turn_id).await
    }

    /// Archive one session(thread) on server side.
    /// Side effects: sends thread/archive RPC call to app-server.
    pub async fn close_session(&self, thread_id: &str) -> Result<(), RpcError> {
        self.runtime.thread_archive(thread_id).await
    }

    /// Borrow underlying runtime for full low-level control.
    /// Allocation: none. Complexity: O(1).
    pub fn runtime(&self) -> &Runtime {
        &self.runtime
    }

    /// Return resolved schema directory used at connection.
    /// Allocation: none. Complexity: O(1).
    pub fn schema_dir(&self) -> &Path {
        &self.schema_dir
    }

    /// Return connect-time client config snapshot.
    /// Allocation: none. Complexity: O(1).
    pub fn config(&self) -> &ClientConfig {
        &self.config
    }

    /// Shutdown child process and background tasks.
    /// Side effects: closes channels and terminates child process.
    pub async fn shutdown(&self) -> Result<(), RuntimeError> {
        self.runtime.shutdown().await
    }
}

/// Pure transform from reusable session config + prompt into one prompt-run request.
/// Allocation: clones config-owned Strings/vectors + one prompt String. Complexity: O(n), n = attachment count + field sizes.
fn session_prompt_params(config: &SessionConfig, prompt: impl Into<String>) -> PromptRunParams {
    PromptRunParams {
        cwd: config.cwd.clone(),
        prompt: prompt.into(),
        model: config.model.clone(),
        effort: Some(config.effort),
        approval_policy: config.approval_policy,
        sandbox_policy: config.sandbox_policy.clone(),
        attachments: config.attachments.clone(),
        timeout: config.timeout,
    }
}

/// Pure transform from reusable profile + turn input into one prompt-run request.
/// Allocation: moves profile-owned Strings/vectors + one prompt String. Complexity: O(n), n = attachment count + field sizes.
fn profile_to_prompt_params(
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
        attachments: profile.attachments,
        timeout: profile.timeout,
    }
}

/// Pure transform from session defaults into thread-start/resume overrides.
/// Allocation: clones Strings/policy payloads from config. Complexity: O(n), n = field sizes.
fn session_thread_start_params(config: &SessionConfig) -> ThreadStartParams {
    ThreadStartParams {
        model: config.model.clone(),
        cwd: Some(config.cwd.clone()),
        approval_policy: Some(config.approval_policy),
        sandbox_policy: Some(config.sandbox_policy.clone()),
    }
}

/// Merge session-default hooks with per-call profile hooks.
/// Ordering is overlay-first to let per-call hooks win on duplicate names.
fn merge_hook_configs(
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

fn ensure_session_open_for_prompt(closed: bool) -> Result<(), PromptRunError> {
    if closed {
        return Err(PromptRunError::Rpc(RpcError::InvalidRequest(
            "session is closed".to_owned(),
        )));
    }
    Ok(())
}

fn ensure_session_open_for_rpc(closed: bool) -> Result<(), RpcError> {
    if closed {
        return Err(RpcError::InvalidRequest("session is closed".to_owned()));
    }
    Ok(())
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum ClientError {
    #[error("failed to read current directory: {0}")]
    CurrentDir(String),

    #[error("schema dir not found: {0}")]
    SchemaDirNotFound(String),

    #[error("schema dir is not a directory: {0}")]
    SchemaDirNotDirectory(String),

    #[error("initialize response missing userAgent")]
    MissingInitializeUserAgent,

    #[error("initialize response has unsupported userAgent format: {0}")]
    InvalidInitializeUserAgent(String),

    #[error("incompatible codex runtime version: detected={detected} required>={required} userAgent={user_agent}")]
    IncompatibleCodexVersion {
        detected: String,
        required: String,
        user_agent: String,
    },

    #[error("runtime error: {0}")]
    Runtime(#[from] RuntimeError),
}

fn validate_runtime_compatibility(
    runtime: &Runtime,
    guard: &CompatibilityGuard,
) -> Result<(), ClientError> {
    if !guard.require_initialize_user_agent && guard.min_codex_version.is_none() {
        return Ok(());
    }

    let user_agent = runtime.server_user_agent();
    if user_agent.is_none() {
        if guard.require_initialize_user_agent {
            return Err(ClientError::MissingInitializeUserAgent);
        }
        return Ok(());
    }

    let user_agent = user_agent.expect("checked is_some");
    let (product, version) = parse_initialize_user_agent(&user_agent)
        .ok_or_else(|| ClientError::InvalidInitializeUserAgent(user_agent.clone()))?;
    let is_codex_product = product.starts_with("Codex ");

    if is_codex_product {
        if let Some(min_required) = guard.min_codex_version {
            if version < min_required {
                return Err(ClientError::IncompatibleCodexVersion {
                    detected: version.to_string(),
                    required: min_required.to_string(),
                    user_agent,
                });
            }
        }
    }

    Ok(())
}

fn parse_initialize_user_agent(value: &str) -> Option<(String, SemVerTriplet)> {
    let slash = value.find('/')?;
    let product = value.get(..slash)?.trim().to_owned();
    if product.is_empty() {
        return None;
    }

    let version_part = value
        .get(slash + 1..)?
        .chars()
        .take_while(|ch| ch.is_ascii_digit() || *ch == '.')
        .collect::<String>();
    let mut parts = version_part.split('.');
    let major = parts.next()?.parse::<u32>().ok()?;
    let minor = parts.next()?.parse::<u32>().ok()?;
    let patch = parts.next()?.parse::<u32>().ok()?;

    Some((product, SemVerTriplet::new(major, minor, patch)))
}

fn resolve_default_schema_dir(cwd: &Path, package_default: &Path) -> Result<PathBuf, ClientError> {
    let cwd_default = cwd.join(DEFAULT_SCHEMA_RELATIVE_DIR);
    if cwd_default.exists() {
        return validate_schema_dir(cwd_default);
    }
    if package_default != cwd_default && package_default.exists() {
        return validate_schema_dir(package_default.to_path_buf());
    }
    validate_schema_dir(cwd_default)
}

fn package_schema_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(PACKAGE_SCHEMA_RELATIVE_FROM_CRATE)
}

fn validate_schema_dir(path: PathBuf) -> Result<PathBuf, ClientError> {
    if !path.exists() {
        return Err(ClientError::SchemaDirNotFound(
            path.to_string_lossy().to_string(),
        ));
    }
    if !path.is_dir() {
        return Err(ClientError::SchemaDirNotDirectory(
            path.to_string_lossy().to_string(),
        ));
    }
    Ok(path)
}

#[cfg(test)]
mod tests;
