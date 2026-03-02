use std::future::Future;

use crate::api::PromptRunResult;
use crate::client::RunProfile;
use crate::ergonomic::{quick_run, quick_run_with_profile};
use crate::ports::outbound::codex_gateway_port::CodexGatewayPort;

#[derive(Clone, Default)]
pub struct TokioCodexGateway;

impl TokioCodexGateway {
    pub fn new() -> Self {
        Self
    }

    fn block_on<T, E, F>(&self, fut: F) -> Result<T, String>
    where
        E: std::fmt::Display,
        F: Future<Output = Result<T, E>>,
    {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|err| format!("failed to build tokio runtime: {err}"))?;

        runtime.block_on(fut).map_err(|err| err.to_string())
    }
}

impl CodexGatewayPort for TokioCodexGateway {
    fn quick_run(&self, cwd: &str, prompt: &str) -> Result<PromptRunResult, String> {
        self.block_on(quick_run(cwd.to_owned(), prompt.to_owned()))
    }

    fn quick_run_with_profile(
        &self,
        cwd: &str,
        prompt: &str,
        profile: RunProfile,
    ) -> Result<PromptRunResult, String> {
        self.block_on(quick_run_with_profile(
            cwd.to_owned(),
            prompt.to_owned(),
            profile,
        ))
    }
}
