use super::*;

#[tokio::test(flavor = "current_thread")]
async fn post_approval_rejects_cross_session_approval_owner_mismatch() {
    let runtime = spawn_mock_runtime().await;
    let adapter = WebAdapter::spawn(runtime.clone(), WebAdapterConfig::default())
        .await
        .expect("adapter spawn");

    let session_a = adapter
        .create_session(
            "tenant_a",
            CreateSessionRequest {
                artifact_id: "doc:approval-a".to_owned(),
                model: None,
                thread_id: None,
            },
        )
        .await
        .expect("create session a");
    let session_b = adapter
        .create_session(
            "tenant_a",
            CreateSessionRequest {
                artifact_id: "doc:approval-b".to_owned(),
                model: None,
                thread_id: None,
            },
        )
        .await
        .expect("create session b");

    let mut approvals_a = adapter
        .subscribe_session_approvals("tenant_a", &session_a.session_id)
        .await
        .expect("subscribe approvals for session a");
    let mut events_a = adapter
        .subscribe_session_events("tenant_a", &session_a.session_id)
        .await
        .expect("subscribe events for session a");
    let mut events_b = adapter
        .subscribe_session_events("tenant_a", &session_b.session_id)
        .await
        .expect("subscribe events for session b");

    adapter
        .create_turn(
            "tenant_a",
            &session_a.session_id,
            CreateTurnRequest {
                task: turn_task("need_approval"),
            },
        )
        .await
        .expect("create turn requiring approval");

    let request = timeout(Duration::from_secs(2), approvals_a.recv())
        .await
        .expect("approval timeout")
        .expect("approval channel closed");

    let err = adapter
        .post_approval(
            "tenant_a",
            &session_b.session_id,
            &request.approval_id,
            ApprovalResponsePayload {
                decision: Some(Value::String("decline".to_owned())),
                result: None,
            },
        )
        .await
        .expect_err("cross-session approval must be forbidden");
    assert_eq!(err, WebError::Forbidden);

    adapter
        .post_approval(
            "tenant_a",
            &session_a.session_id,
            &request.approval_id,
            ApprovalResponsePayload {
                decision: Some(Value::String("decline".to_owned())),
                result: None,
            },
        )
        .await
        .expect("owner session should still approve");

    loop {
        let envelope = timeout(Duration::from_secs(2), events_a.recv())
            .await
            .expect("ack timeout")
            .expect("event channel closed");
        if envelope.method.as_deref() == Some("approval/ack") {
            assert_eq!(
                envelope.thread_id.as_deref(),
                Some(session_a.thread_id.as_str())
            );
            break;
        }
    }

    assert_no_thread_leak(
        &mut events_b,
        &session_a.thread_id,
        Duration::from_millis(250),
    )
    .await;

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn post_approval_rejects_reused_approval_id_after_successful_response() {
    let runtime = spawn_mock_runtime().await;
    let adapter = WebAdapter::spawn(runtime.clone(), WebAdapterConfig::default())
        .await
        .expect("adapter spawn");

    let session = adapter
        .create_session(
            "tenant_a",
            CreateSessionRequest {
                artifact_id: "doc:approval-reuse".to_owned(),
                model: None,
                thread_id: None,
            },
        )
        .await
        .expect("create session");

    let mut approvals = adapter
        .subscribe_session_approvals("tenant_a", &session.session_id)
        .await
        .expect("subscribe approvals");

    adapter
        .create_turn(
            "tenant_a",
            &session.session_id,
            CreateTurnRequest {
                task: turn_task("need_approval"),
            },
        )
        .await
        .expect("create turn requiring approval");

    let request = timeout(Duration::from_secs(2), approvals.recv())
        .await
        .expect("approval timeout")
        .expect("approval channel closed");

    adapter
        .post_approval(
            "tenant_a",
            &session.session_id,
            &request.approval_id,
            ApprovalResponsePayload {
                decision: Some(Value::String("decline".to_owned())),
                result: None,
            },
        )
        .await
        .expect("first approval response");

    let err = adapter
        .post_approval(
            "tenant_a",
            &session.session_id,
            &request.approval_id,
            ApprovalResponsePayload {
                decision: Some(Value::String("decline".to_owned())),
                result: None,
            },
        )
        .await
        .expect_err("approval id must be single-use");
    assert_eq!(err, WebError::InvalidApproval);

    runtime.shutdown().await.expect("shutdown");
}
