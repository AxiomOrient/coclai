use std::path::PathBuf;
use std::sync::Arc;

use crate::plugin::{PostHook, PreHook};
use crate::runtime::hooks::RuntimeHookConfig;
use crate::runtime::InitializeCapabilities;

use super::CompatibilityGuard;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientConfig {
    pub cli_bin: PathBuf,
    pub compatibility_guard: CompatibilityGuard,
    pub initialize_capabilities: InitializeCapabilities,
    pub hooks: RuntimeHookConfig,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            cli_bin: PathBuf::from("codex"),
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
}
