use crate::runtime::{
    Client, ClientError, PromptRunError, PromptRunResult, RunProfile, RuntimeError,
};
use thiserror::Error;

/// Error model for one-shot convenience calls.
/// Side effects are explicit: run errors can carry shutdown errors.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum QuickRunError {
    #[error("failed to connect codex runtime: {0}")]
    Connect(#[from] ClientError),
    #[error("prompt run failed: {run}; shutdown_error={shutdown:?}")]
    Run {
        run: PromptRunError,
        shutdown: Option<RuntimeError>,
    },
    #[error("runtime shutdown failed after successful run: {0}")]
    Shutdown(#[from] RuntimeError),
}

/// One-shot convenience:
/// connect -> run(default profile) -> shutdown
pub async fn quick_run(
    cwd: impl Into<String>,
    prompt: impl Into<String>,
) -> Result<PromptRunResult, QuickRunError> {
    quick_run_impl(cwd.into(), prompt.into(), None).await
}

/// One-shot convenience with explicit profile:
/// connect -> run(profile) -> shutdown
pub async fn quick_run_with_profile(
    cwd: impl Into<String>,
    prompt: impl Into<String>,
    profile: RunProfile,
) -> Result<PromptRunResult, QuickRunError> {
    quick_run_impl(cwd.into(), prompt.into(), Some(profile)).await
}

async fn quick_run_impl(
    cwd: String,
    prompt: String,
    profile: Option<RunProfile>,
) -> Result<PromptRunResult, QuickRunError> {
    let client = Client::connect_default().await?;
    let run_result = match profile {
        Some(profile) => client.run_with_profile(cwd, prompt, profile).await,
        None => client.run(cwd, prompt).await,
    };
    let shutdown_result = client.shutdown().await;
    fold_quick_run(run_result, shutdown_result)
}

pub(crate) fn fold_quick_run(
    run_result: Result<PromptRunResult, PromptRunError>,
    shutdown_result: Result<(), RuntimeError>,
) -> Result<PromptRunResult, QuickRunError> {
    match (run_result, shutdown_result) {
        (Ok(output), Ok(())) => Ok(output),
        (Ok(_), Err(shutdown)) => Err(QuickRunError::Shutdown(shutdown)),
        (Err(run), Ok(())) => Err(QuickRunError::Run {
            run,
            shutdown: None,
        }),
        (Err(run), Err(shutdown)) => Err(QuickRunError::Run {
            run,
            shutdown: Some(shutdown),
        }),
    }
}
