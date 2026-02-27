use std::path::{Path, PathBuf};
use std::sync::Arc;

use coclai_plugin_core::{PostHook, PreHook};

use crate::hooks::RuntimeHookConfig;

use super::{ClientError, CompatibilityGuard};

pub const DEFAULT_SCHEMA_RELATIVE_DIR: &str = "SCHEMAS/app-server/active";
pub const SCHEMA_DIR_ENV: &str = "APP_SERVER_SCHEMA_DIR";
const PACKAGE_SCHEMA_RELATIVE_FROM_CRATE: &str = "../../SCHEMAS/app-server/active";

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

pub(super) fn resolve_default_schema_dir(
    cwd: &Path,
    package_default: &Path,
) -> Result<PathBuf, ClientError> {
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

pub(super) fn validate_schema_dir(path: PathBuf) -> Result<PathBuf, ClientError> {
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
