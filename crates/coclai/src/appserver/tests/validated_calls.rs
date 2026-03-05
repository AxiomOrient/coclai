use super::*;

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

#[tokio::test(flavor = "current_thread")]
async fn request_json_rejects_empty_method_name_before_send() {
    let app = connect_real_appserver().await;

    let err = app
        .request_json("", json!({}))
        .await
        .expect_err("empty method name must fail request validation");
    assert!(matches!(err, RpcError::InvalidRequest(_)));

    app.shutdown().await.expect("shutdown");
}
