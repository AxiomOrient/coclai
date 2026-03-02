use serde_json::{json, Value};

use crate::api::PromptRunResult;
use crate::client::RunProfile;
use crate::ergonomic::{Workflow, WorkflowConfig};

pub fn make_workflow_config(cwd: String, profile: Option<RunProfile>) -> WorkflowConfig {
    let mut config = WorkflowConfig::new(cwd);
    if let Some(profile) = profile {
        config = config.with_run_profile(profile);
    }
    config
}

pub fn render_connect_result(workflow_id: &str, cwd: &str) -> Value {
    json!({
        "workflow_id": workflow_id,
        "cwd": cwd,
    })
}

pub async fn execute_workflow_run(
    config: WorkflowConfig,
    prompt: String,
) -> Result<PromptRunResult, String> {
    let workflow = Workflow::connect(config)
        .await
        .map_err(|err| format!("workflow connect failed: {err}"))?;
    let run_result = workflow.run(prompt).await;
    let shutdown_result = workflow.shutdown().await;
    match (run_result, shutdown_result) {
        (Ok(output), Ok(())) => Ok(output),
        (Ok(_), Err(shutdown_err)) => Err(format!(
            "workflow shutdown failed after run: {shutdown_err}"
        )),
        (Err(run_err), Ok(())) => Err(format!("workflow run failed: {run_err}")),
        (Err(run_err), Err(shutdown_err)) => Err(format!(
            "workflow run failed: {run_err}; shutdown failed: {shutdown_err}"
        )),
    }
}

pub fn render_run_result(workflow_id: &str, output: PromptRunResult) -> Value {
    json!({
        "workflow_id": workflow_id,
        "thread_id": output.thread_id,
        "turn_id": output.turn_id,
        "assistant_text": output.assistant_text,
    })
}

pub async fn execute_session_setup(config: WorkflowConfig) -> Result<String, String> {
    let workflow = Workflow::connect(config)
        .await
        .map_err(|err| format!("workflow connect failed: {err}"))?;
    let session = workflow
        .setup_session()
        .await
        .map_err(|err| format!("workflow setup_session failed: {err}"))?;
    let thread_id = session.thread_id.clone();
    workflow
        .shutdown()
        .await
        .map_err(|err| format!("workflow shutdown failed: {err}"))?;
    Ok(thread_id)
}

pub fn render_session_setup_result(workflow_id: &str, thread_id: &str) -> Value {
    json!({
        "workflow_id": workflow_id,
        "thread_id": thread_id,
    })
}
