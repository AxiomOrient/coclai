use crate::runtime::turn_output::parse_thread_id;
use crate::runtime::{ThreadReadParams, ThreadReadResponse};
use serde_json::{json, Value};

use super::*;

fn thread_start_params() -> Value {
    json!({
        "cwd": std::env::current_dir()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|_| ".".to_owned()),
        "approvalPolicy": "never",
        "sandbox": "read-only"
    })
}

async fn connect_real_appserver() -> AppServer {
    AppServer::connect_default()
        .await
        .expect("connect codex app-server")
}

async fn start_thread(app: &AppServer) -> String {
    let response = app
        .request_json(methods::THREAD_START, thread_start_params())
        .await
        .expect("thread/start request");
    parse_thread_id(&response).expect("thread/start must return thread id")
}

async fn archive_thread_best_effort(app: &AppServer, thread_id: &str) {
    let _ = app
        .request_json(
            methods::THREAD_ARCHIVE,
            json!({
                "threadId": thread_id
            }),
        )
        .await;
}

mod contract;
mod server_requests;
mod validated_calls;
