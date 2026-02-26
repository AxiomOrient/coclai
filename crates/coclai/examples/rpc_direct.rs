use coclai::{rpc_methods, AppServer};
use serde_json::json;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cwd = std::env::var("COCLAI_CWD").unwrap_or_else(|_| ".".to_owned());
    let prompt = std::env::var("COCLAI_PROMPT").unwrap_or_else(|_| "say hello".to_owned());

    let app = AppServer::connect_default().await?;

    let thread = app
        .request_json(
            rpc_methods::THREAD_START,
            json!({
                "cwd": cwd,
                "approvalPolicy": "never",
                "sandboxPolicy": { "mode": "read-only" }
            }),
        )
        .await?;

    let thread_id = thread
        .pointer("/thread/id")
        .and_then(|v| v.as_str())
        .or_else(|| thread.get("threadId").and_then(|v| v.as_str()))
        .ok_or("thread/start missing thread id")?
        .to_owned();

    let turn = app
        .request_json(
            rpc_methods::TURN_START,
            json!({
                "threadId": thread_id,
                "input": [{"type":"text","text": prompt}],
            }),
        )
        .await?;

    println!("turn: {turn}");

    app.shutdown().await?;
    Ok(())
}
