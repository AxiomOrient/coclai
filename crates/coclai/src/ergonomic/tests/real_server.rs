use super::super::*;
use tokio::time::{sleep, Duration as TokioDuration};

const MAX_REAL_SERVER_RETRIES: usize = 5;
const QUICK_RUN_ATTEMPT_TIMEOUT: TokioDuration = TokioDuration::from_secs(45);
const WORKFLOW_RUN_ATTEMPT_TIMEOUT: TokioDuration = TokioDuration::from_secs(60);

fn is_transient_real_server_error(err: &str) -> bool {
    let e = err.to_ascii_lowercase();
    e.contains("stream disconnected")
        || e.contains("error sending request")
        || e.contains("timed out")
        || e.contains("connection reset")
        || e.contains("connection refused")
}

async fn backoff_after_attempt(attempt: usize) {
    let seconds = (attempt as u64).min(5);
    sleep(TokioDuration::from_secs(seconds)).await;
}

async fn quick_run_attempt(cwd: String, prompt: &'static str) -> Result<PromptRunResult, String> {
    tokio::time::timeout(QUICK_RUN_ATTEMPT_TIMEOUT, quick_run(cwd, prompt))
        .await
        .map_err(|_| format!("quick_run attempt timed out after {QUICK_RUN_ATTEMPT_TIMEOUT:?}"))?
        .map_err(|err| format!("{err:?}"))
}

async fn workflow_run_attempt(cwd: String) -> Result<PromptRunResult, String> {
    tokio::time::timeout(WORKFLOW_RUN_ATTEMPT_TIMEOUT, async move {
        let config = WorkflowConfig::new(cwd).with_timeout(Duration::from_secs(120));
        let workflow = Workflow::connect(config)
            .await
            .expect("workflow connect with real codex server");
        let run_result = workflow
            .run("Reply with one short sentence about Rust testing.")
            .await;
        let shutdown_result = workflow.shutdown().await;

        match run_result {
            Ok(result) => {
                if let Err(shutdown_err) = shutdown_result {
                    panic!("workflow shutdown failed after successful run: {shutdown_err}");
                }
                Ok(result)
            }
            Err(err) => {
                if let Err(shutdown_err) = shutdown_result {
                    eprintln!("warning: shutdown failed after run error: {shutdown_err}");
                }
                Err(format!("{err:?}"))
            }
        }
    })
    .await
    .map_err(|_| format!("workflow attempt timed out after {WORKFLOW_RUN_ATTEMPT_TIMEOUT:?}"))?
}

#[tokio::test(flavor = "current_thread")]
async fn quick_run_executes_prompt_against_real_codex_server() {
    let cwd = std::env::current_dir()
        .expect("cwd")
        .to_string_lossy()
        .to_string();
    let prompt = "Reply with one short sentence about this workspace.";

    let mut last_err = None;
    let mut out = None;
    for attempt in 1..=MAX_REAL_SERVER_RETRIES {
        match quick_run_attempt(cwd.clone(), prompt).await {
            Ok(result) => {
                out = Some(result);
                break;
            }
            Err(err_text) => {
                last_err = Some(err_text.clone());
                if !is_transient_real_server_error(&err_text) {
                    break;
                }
                if attempt < MAX_REAL_SERVER_RETRIES {
                    backoff_after_attempt(attempt).await;
                }
            }
        }
    }

    let out = out.unwrap_or_else(|| {
        panic!(
            "quick_run with real codex server failed after retries({MAX_REAL_SERVER_RETRIES}): {:?}",
            last_err
        )
    });
    assert!(!out.thread_id.is_empty());
    assert!(!out.turn_id.is_empty());
    assert!(!out.assistant_text.trim().is_empty());
}

#[tokio::test(flavor = "current_thread")]
async fn workflow_run_executes_prompt_against_real_codex_server() {
    let cwd = std::env::current_dir()
        .expect("cwd")
        .to_string_lossy()
        .to_string();

    let mut last_err = None;
    let mut out = None;
    for attempt in 1..=MAX_REAL_SERVER_RETRIES {
        match workflow_run_attempt(cwd.clone()).await {
            Ok(result) => {
                out = Some(result);
                break;
            }
            Err(err_text) => {
                last_err = Some(err_text.clone());
                if !is_transient_real_server_error(&err_text) {
                    break;
                }
                if attempt < MAX_REAL_SERVER_RETRIES {
                    backoff_after_attempt(attempt).await;
                }
            }
        }
    }

    let out = out.unwrap_or_else(|| {
        panic!(
            "workflow run failed against real codex server after retries({MAX_REAL_SERVER_RETRIES}): {:?}",
            last_err
        )
    });
    assert!(!out.thread_id.is_empty());
    assert!(!out.turn_id.is_empty());
    assert!(!out.assistant_text.trim().is_empty());
}
