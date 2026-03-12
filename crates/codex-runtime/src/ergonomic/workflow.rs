use crate::runtime::{
    Client, ClientError, PromptRunError, PromptRunResult, RunProfile, RuntimeError, Session,
    SessionConfig,
};

use crate::ergonomic::WorkflowConfig;

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
        self.run_with_profile(prompt, self.config.run_profile.clone())
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
        self.setup_session_with_profile(self.config.run_profile.clone())
            .await
    }

    /// Start one session with explicit profile override.
    pub async fn setup_session_with_profile(
        &self,
        profile: RunProfile,
    ) -> Result<Session, PromptRunError> {
        self.client
            .start_session(SessionConfig::from_profile(
                self.config.cwd.clone(),
                profile,
            ))
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
