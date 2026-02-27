use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::api::{PromptRunError, PromptRunParams, PromptRunResult};
use crate::errors::{RpcError, RuntimeError};
use crate::runtime::{Runtime, RuntimeConfig, SchemaGuardConfig};
use crate::transport::StdioProcessSpec;

mod compat_guard;
mod config;
mod profile;
mod session;

pub use compat_guard::{CompatibilityGuard, SemVerTriplet};
pub use config::{ClientConfig, DEFAULT_SCHEMA_RELATIVE_DIR, SCHEMA_DIR_ENV};
pub use profile::{RunProfile, SessionConfig};
pub use session::Session;

use compat_guard::validate_runtime_compatibility;
use profile::session_thread_start_params;

#[derive(Clone)]
pub struct Client {
    runtime: Runtime,
    config: ClientConfig,
    schema_dir: PathBuf,
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
        if let Err(compatibility) =
            validate_runtime_compatibility(&runtime, &config.compatibility_guard)
        {
            if let Err(shutdown) = runtime.shutdown().await {
                return Err(ClientError::CompatibilityValidationWithShutdown {
                    compatibility: Box::new(compatibility),
                    shutdown,
                });
            }
            return Err(compatibility);
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
                profile::profile_to_prompt_params(cwd.into(), prompt, profile),
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

        Ok(Session::new(self.runtime.clone(), thread.thread_id, config))
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

        Ok(Session::new(self.runtime.clone(), thread.thread_id, config))
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
                profile::profile_to_prompt_params(cwd.into(), prompt, profile),
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

    #[error(
        "compatibility validation failed: {compatibility}; runtime shutdown failed: {shutdown}"
    )]
    CompatibilityValidationWithShutdown {
        compatibility: Box<ClientError>,
        shutdown: RuntimeError,
    },

    #[error("runtime error: {0}")]
    Runtime(#[from] RuntimeError),
}

#[cfg(test)]
fn parse_initialize_user_agent(value: &str) -> Option<(String, SemVerTriplet)> {
    compat_guard::parse_initialize_user_agent(value)
}

#[cfg(test)]
fn resolve_default_schema_dir(cwd: &Path, package_default: &Path) -> Result<PathBuf, ClientError> {
    config::resolve_default_schema_dir(cwd, package_default)
}

#[cfg(test)]
fn validate_schema_dir(path: PathBuf) -> Result<PathBuf, ClientError> {
    config::validate_schema_dir(path)
}

#[cfg(test)]
fn session_prompt_params(config: &SessionConfig, prompt: impl Into<String>) -> PromptRunParams {
    profile::session_prompt_params(config, prompt)
}

#[cfg(test)]
fn profile_to_prompt_params(
    cwd: String,
    prompt: impl Into<String>,
    profile: RunProfile,
) -> PromptRunParams {
    profile::profile_to_prompt_params(cwd, prompt, profile)
}

#[cfg(test)]
fn ensure_session_open_for_prompt(closed: bool) -> Result<(), PromptRunError> {
    session::ensure_session_open_for_prompt(closed)
}

#[cfg(test)]
fn ensure_session_open_for_rpc(closed: bool) -> Result<(), RpcError> {
    session::ensure_session_open_for_rpc(closed)
}

#[cfg(test)]
mod tests;
