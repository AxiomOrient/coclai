use coclai::{
    ApprovalPolicy, ReasoningEffort, SandboxPolicy, SandboxPreset, Workflow, WorkflowConfig,
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

    let config = WorkflowConfig::new(cwd.clone())
        .with_model("gpt-5-codex")
        .with_effort(ReasoningEffort::High)
        .with_approval_policy(ApprovalPolicy::OnRequest)
        .with_sandbox_policy(SandboxPolicy::Preset(SandboxPreset::WorkspaceWrite {
            writable_roots: vec![cwd],
            network_access: false,
        }));

    let workflow = Workflow::connect(config).await?;
    let out = workflow.run(prompt).await?;
    println!("{}", out.assistant_text);
    workflow.shutdown().await?;
    Ok(())
}
