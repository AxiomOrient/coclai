use super::*;

#[tokio::test(flavor = "current_thread")]
async fn take_server_requests_is_exclusive() {
    let app = connect_real_appserver().await;

    let _first = app
        .take_server_requests()
        .await
        .expect("first receiver ownership must succeed");
    let err = app
        .take_server_requests()
        .await
        .expect_err("second ownership claim must fail");
    assert_eq!(err, RuntimeError::ServerRequestReceiverTaken);

    app.shutdown().await.expect("shutdown");
}
