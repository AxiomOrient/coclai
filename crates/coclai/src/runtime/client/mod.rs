use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use thiserror::Error;

use crate::runtime::api::{tool_use_hooks, PromptRunError, PromptRunParams, PromptRunResult};
use crate::runtime::core::{Runtime, RuntimeConfig};
#[cfg(test)]
use crate::runtime::errors::RpcError;
use crate::runtime::errors::RuntimeError;
use crate::runtime::transport::StdioProcessSpec;

mod compat_guard;
mod config;
mod profile;
mod session;

pub use compat_guard::{CompatibilityGuard, SemVerTriplet};
pub use config::ClientConfig;
pub use profile::{RunProfile, SessionConfig};
pub use session::Session;

use compat_guard::validate_runtime_compatibility;
use profile::{profile_to_prompt_params_with_hooks, session_thread_start_params};

#[derive(Clone)]
pub struct Client {
    runtime: Runtime,
    config: ClientConfig,
    tool_use_loop_started: Arc<AtomicBool>,
}

impl Client {
    /// Connect using default config (default CLI).
    /// Side effects: spawns `<cli_bin> app-server`.
    /// Allocation: runtime buffers + internal channels.
    pub async fn connect_default() -> Result<Self, ClientError> {
        Self::connect(ClientConfig::new()).await
    }

    /// Connect using explicit client config.
    /// Side effects: spawns `<cli_bin> app-server` and validates initialize compatibility guard.
    /// Allocation: runtime buffers + internal channels.
    pub async fn connect(config: ClientConfig) -> Result<Self, ClientError> {
        let mut process = StdioProcessSpec::new(config.cli_bin.clone());
        process.args = vec!["app-server".to_owned()];

        let runtime = Runtime::spawn_local(
            RuntimeConfig::new(process)
                .with_hooks(config.hooks.clone())
                .with_initialize_capabilities(config.initialize_capabilities),
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

        // When pre-tool-use hooks are registered, start a background approval loop that
        // intercepts commandExecution/fileChange approval requests and runs the hooks.
        // The loop takes exclusive ownership of server_request_rx for the runtime's lifetime.
        let tool_use_loop_started = Arc::new(AtomicBool::new(false));
        let client = Self {
            runtime,
            config,
            tool_use_loop_started,
        };
        client.ensure_tool_use_hook_loop(client.config.hooks.has_pre_tool_use_hooks());
        Ok(client)
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
        let (params, hooks) = profile_to_prompt_params_with_hooks(cwd.into(), prompt, profile);
        self.ensure_tool_use_hook_loop(hooks.has_pre_tool_use_hooks());
        self.runtime
            .run_prompt_with_hooks(params, Some(&hooks))
            .await
    }

    /// Start a prepared session and return a reusable handle.
    /// Side effects: sends thread/start RPC call to app-server.
    /// Allocation: clones model/cwd/sandbox into thread-start payload. Complexity: O(n), n = total field sizes.
    pub async fn start_session(&self, config: SessionConfig) -> Result<Session, PromptRunError> {
        self.ensure_tool_use_hook_loop(config.hooks.has_pre_tool_use_hooks());
        let thread = self
            .runtime
            .thread_start_with_hooks(session_thread_start_params(&config), Some(&config.hooks))
            .await?;

        Ok(Session::new(
            self.runtime.clone(),
            thread.thread_id,
            config,
            Arc::clone(&self.tool_use_loop_started),
        ))
    }

    /// Resume an existing session id with prepared defaults.
    /// Side effects: sends thread/resume RPC call to app-server.
    /// Allocation: clones model/cwd/sandbox into thread-resume payload. Complexity: O(n), n = total field sizes.
    pub async fn resume_session(
        &self,
        thread_id: &str,
        config: SessionConfig,
    ) -> Result<Session, PromptRunError> {
        self.ensure_tool_use_hook_loop(config.hooks.has_pre_tool_use_hooks());
        let thread = self
            .runtime
            .thread_resume_with_hooks(
                thread_id,
                session_thread_start_params(&config),
                Some(&config.hooks),
            )
            .await?;

        Ok(Session::new(
            self.runtime.clone(),
            thread.thread_id,
            config,
            Arc::clone(&self.tool_use_loop_started),
        ))
    }

    /// Borrow underlying runtime for full low-level control.
    /// Allocation: none. Complexity: O(1).
    pub fn runtime(&self) -> &Runtime {
        &self.runtime
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

    fn ensure_tool_use_hook_loop(&self, needs_loop: bool) {
        if !needs_loop {
            return;
        }
        if self
            .tool_use_loop_started
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            tokio::spawn(tool_use_hooks::run_tool_use_approval_loop(
                self.runtime.clone(),
            ));
        }
    }
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum ClientError {
    #[error("failed to read current directory: {0}")]
    CurrentDir(String),

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
