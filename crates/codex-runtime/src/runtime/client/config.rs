use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::plugin::{PostHook, PreHook};
use crate::runtime::hooks::RuntimeHookConfig;
use crate::runtime::InitializeCapabilities;

use super::CompatibilityGuard;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientConfig {
    pub cli_bin: PathBuf,
    pub process_env: HashMap<String, String>,
    pub process_cwd: Option<PathBuf>,
    pub app_server_args: Vec<String>,
    pub compatibility_guard: CompatibilityGuard,
    pub initialize_capabilities: InitializeCapabilities,
    pub hooks: RuntimeHookConfig,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            cli_bin: PathBuf::from("codex"),
            process_env: HashMap::new(),
            process_cwd: None,
            app_server_args: Vec::new(),
            compatibility_guard: CompatibilityGuard::default(),
            initialize_capabilities: InitializeCapabilities::default(),
            hooks: RuntimeHookConfig::default(),
        }
    }
}

impl ClientConfig {
    /// Create config with default binary discovery.
    pub fn new() -> Self {
        Self::default()
    }

    /// Override CLI executable path.
    pub fn with_cli_bin(mut self, cli_bin: impl Into<PathBuf>) -> Self {
        self.cli_bin = cli_bin.into();
        self
    }

    /// Replace process environment overrides for the spawned app-server child.
    pub fn with_process_envs(
        mut self,
        process_env: impl IntoIterator<Item = (impl Into<String>, impl Into<String>)>,
    ) -> Self {
        self.process_env = process_env
            .into_iter()
            .map(|(key, value)| (key.into(), value.into()))
            .collect();
        self
    }

    /// Set one process environment override for the spawned app-server child.
    pub fn with_process_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.process_env.insert(key.into(), value.into());
        self
    }

    /// Override the working directory used to spawn the app-server child process.
    pub fn with_process_cwd(mut self, process_cwd: impl Into<PathBuf>) -> Self {
        self.process_cwd = Some(process_cwd.into());
        self
    }

    /// Replace extra args appended after the fixed `app-server` subcommand.
    pub fn with_app_server_args(
        mut self,
        args: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.app_server_args = args.into_iter().map(Into::into).collect();
        self
    }

    /// Add one extra arg appended after the fixed `app-server` subcommand.
    pub fn with_app_server_arg(mut self, arg: impl Into<String>) -> Self {
        self.app_server_args.push(arg.into());
        self
    }

    /// Override runtime compatibility guard policy.
    pub fn with_compatibility_guard(mut self, guard: CompatibilityGuard) -> Self {
        self.compatibility_guard = guard;
        self
    }

    /// Disable compatibility guard checks at connect time.
    pub fn without_compatibility_guard(mut self) -> Self {
        self.compatibility_guard = CompatibilityGuard {
            require_initialize_user_agent: false,
            min_codex_version: None,
        };
        self
    }

    /// Override initialize capability switches.
    pub fn with_initialize_capabilities(
        mut self,
        initialize_capabilities: InitializeCapabilities,
    ) -> Self {
        self.initialize_capabilities = initialize_capabilities;
        self
    }

    /// Opt into Codex experimental app-server methods and fields.
    pub fn enable_experimental_api(mut self) -> Self {
        self.initialize_capabilities = self.initialize_capabilities.enable_experimental_api();
        self
    }

    /// Replace runtime hook configuration.
    pub fn with_hooks(mut self, hooks: RuntimeHookConfig) -> Self {
        self.hooks = hooks;
        self
    }

    /// Register one pre hook on client runtime config.
    pub fn with_pre_hook(mut self, hook: Arc<dyn PreHook>) -> Self {
        self.hooks.pre_hooks.push(hook);
        self
    }

    /// Register one post hook on client runtime config.
    pub fn with_post_hook(mut self, hook: Arc<dyn PostHook>) -> Self {
        self.hooks.post_hooks.push(hook);
        self
    }

    /// Register one pre-tool-use hook on client runtime config.
    /// When at least one pre-tool-use hook is registered, the runtime manages the
    /// approval channel internally and auto-escalates ApprovalPolicy → Untrusted.
    /// Allocation: amortized O(1) push. Complexity: O(1).
    pub fn with_pre_tool_use_hook(mut self, hook: Arc<dyn PreHook>) -> Self {
        self.hooks.pre_tool_use_hooks.push(hook);
        self
    }
}
