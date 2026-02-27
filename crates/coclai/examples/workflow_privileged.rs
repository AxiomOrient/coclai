use coclai::{
    ApprovalPolicy, ReasoningEffort, RunProfile, SandboxPolicy, SandboxPreset, Workflow,
    WorkflowConfig,
};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cwd = std::env::var("COCLAI_CWD").unwrap_or_else(|_| {
        std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_default()
    });
    let prompt = std::env::var("COCLAI_PROMPT")
        .unwrap_or_else(|_| "README.md를 읽고 핵심 3가지를 정리해줘".to_owned());

    let resolved_cwd = WorkflowConfig::new(cwd).cwd;
    let run_profile = RunProfile::new()
        .with_model("gpt-5-codex")
        .with_effort(ReasoningEffort::High)
        .with_approval_policy(ApprovalPolicy::OnRequest)
        .with_sandbox_policy(SandboxPolicy::Preset(SandboxPreset::WorkspaceWrite {
            writable_roots: vec![resolved_cwd.clone()],
            network_access: false,
        }))
        .allow_privileged_escalation();
    let config = WorkflowConfig::new(resolved_cwd).with_run_profile(run_profile);

    let workflow = match Workflow::connect(config).await {
        Ok(workflow) => workflow,
        Err(err) => return Err(Box::<dyn std::error::Error>::from(err)),
    };
    let run_result = workflow.run(prompt).await;
    let shutdown_result = workflow.shutdown().await;

    let out = match run_result {
        Ok(out) => out,
        Err(run_err) => {
            if let Err(shutdown_err) = shutdown_result {
                eprintln!("warning: workflow shutdown failed after run error: {shutdown_err}");
            }
            return Err(Box::<dyn std::error::Error>::from(run_err));
        }
    };
    if let Err(shutdown_err) = shutdown_result {
        return Err(Box::<dyn std::error::Error>::from(shutdown_err));
    }
    println!("{}", out.assistant_text);
    Ok(())
}
