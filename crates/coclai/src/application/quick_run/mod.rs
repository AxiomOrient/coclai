use serde_json::{json, Value};

use crate::client::RunProfile;
use crate::ports::outbound::codex_gateway_port::CodexGatewayPort;

fn render_prompt_result_json(thread_id: String, turn_id: String, assistant_text: String) -> Value {
    json!({
        "thread_id": thread_id,
        "turn_id": turn_id,
        "assistant_text": assistant_text,
    })
}

pub fn execute_quick_run(
    gateway: &(dyn CodexGatewayPort + Send + Sync),
    cwd: &str,
    prompt: &str,
) -> Result<Value, String> {
    let output = gateway.quick_run(cwd, prompt)?;
    Ok(render_prompt_result_json(
        output.thread_id,
        output.turn_id,
        output.assistant_text,
    ))
}

pub fn execute_quick_run_with_profile(
    gateway: &(dyn CodexGatewayPort + Send + Sync),
    cwd: &str,
    prompt: &str,
    profile: RunProfile,
) -> Result<Value, String> {
    let output = gateway.quick_run_with_profile(cwd, prompt, profile)?;
    Ok(render_prompt_result_json(
        output.thread_id,
        output.turn_id,
        output.assistant_text,
    ))
}
