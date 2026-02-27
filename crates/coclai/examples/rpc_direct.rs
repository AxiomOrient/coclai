use std::time::{Duration, Instant};

use coclai::runtime::turn_output::{parse_thread_id, parse_turn_id, AssistantTextCollector};
use coclai::{rpc_methods, AppServer};
use serde_json::json;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cwd = std::env::var("COCLAI_CWD").unwrap_or_else(|_| {
        std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| ".".to_owned())
    });
    let prompt = std::env::var("COCLAI_PROMPT").unwrap_or_else(|_| "say hello".to_owned());

    let app = AppServer::connect_default().await?;
    let run_result: Result<(), Box<dyn std::error::Error>> = async {
        let mut live_rx = app.runtime().subscribe_live();

        let thread = app
            .request_json(
                rpc_methods::THREAD_START,
                json!({
                    "cwd": cwd,
                    "approvalPolicy": "never",
                    "sandbox": "read-only"
                }),
            )
            .await?;
        let thread_id = parse_thread_id(&thread).ok_or("thread/start missing thread id")?;

        let turn = app
            .request_json(
                rpc_methods::TURN_START,
                json!({
                    "threadId": thread_id,
                    "input": [{"type":"text","text": prompt}],
                }),
            )
            .await?;
        let turn_id = parse_turn_id(&turn).ok_or("turn/start missing turn id")?;

        let mut collector = AssistantTextCollector::new();
        let deadline = Instant::now() + Duration::from_secs(90);
        loop {
            let now = Instant::now();
            if now >= deadline {
                return Err("timed out while waiting turn completion".into());
            }
            let remaining = deadline.saturating_duration_since(now);
            let envelope = match tokio::time::timeout(remaining, live_rx.recv()).await {
                Ok(Ok(envelope)) => envelope,
                Ok(Err(err)) => return Err(Box::<dyn std::error::Error>::from(err)),
                Err(_) => return Err("timed out while waiting turn completion".into()),
            };
            if envelope.thread_id.as_deref() != Some(thread_id.as_str()) {
                continue;
            }
            if envelope.turn_id.as_deref() != Some(turn_id.as_str()) {
                continue;
            }

            collector.push_envelope(&envelope);
            match envelope.method.as_deref() {
                Some("turn/completed") => {
                    let text = collector.into_text();
                    println!("turn_id: {turn_id}");
                    println!("assistant: {text}");
                    break;
                }
                Some("turn/failed") | Some("turn/cancelled") => {
                    return Err(format!(
                        "turn ended with terminal state: {}",
                        envelope.method.as_deref().unwrap_or("unknown")
                    )
                    .into());
                }
                _ => {}
            }
        }
        Ok(())
    }
    .await;

    let shutdown_result = app.shutdown().await;
    if let Err(run_err) = run_result {
        let _ = shutdown_result;
        return Err(run_err);
    }
    shutdown_result?;
    Ok(())
}
