use crate::{ThreadReadParams, ThreadReadResponse};
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

fn parse_thread_id(response: &Value) -> Option<&str> {
    response
        .pointer("/thread/id")
        .and_then(Value::as_str)
        .or_else(|| response.get("threadId").and_then(Value::as_str))
        .or_else(|| response.get("id").and_then(Value::as_str))
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
    parse_thread_id(&response)
        .map(ToOwned::to_owned)
        .expect("thread/start must return thread id")
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

#[tokio::test(flavor = "current_thread")]
async fn request_json_thread_start_returns_thread_id() {
    let app = connect_real_appserver().await;

    let thread_id = start_thread(&app).await;
    assert!(!thread_id.is_empty());

    archive_thread_best_effort(&app, &thread_id).await;
    app.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn request_typed_thread_read_returns_started_thread() {
    let app = connect_real_appserver().await;

    let thread_id = start_thread(&app).await;
    let read: ThreadReadResponse = app
        .request_typed(
            methods::THREAD_READ,
            ThreadReadParams {
                thread_id: thread_id.clone(),
                include_turns: Some(false),
            },
        )
        .await
        .expect("typed thread/read");
    assert_eq!(read.thread.id, thread_id);

    archive_thread_best_effort(&app, &read.thread.id).await;
    app.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn request_json_rejects_invalid_known_params_before_send() {
    let app = connect_real_appserver().await;

    let err = app
        .request_json(methods::TURN_INTERRUPT, json!({"threadId":"thr"}))
        .await
        .expect_err("missing turnId must fail validation");
    assert!(matches!(err, RpcError::InvalidRequest(_)));

    app.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn notify_json_rejects_invalid_known_params_before_send() {
    let app = connect_real_appserver().await;

    let err = app
        .notify_json(methods::TURN_INTERRUPT, json!({"threadId":"thr"}))
        .await
        .expect_err("missing turnId must fail validation");
    assert!(matches!(err, RuntimeError::InvalidConfig(_)));

    app.shutdown().await.expect("shutdown");
}

#[test]
fn method_constants_are_stable() {
    assert_eq!(methods::THREAD_START, "thread/start");
    assert_eq!(methods::TURN_INTERRUPT, "turn/interrupt");
}
