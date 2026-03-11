use super::*;

#[tokio::test(flavor = "current_thread")]
async fn approval_roundtrip_via_post_approval() {
    let runtime = spawn_mock_runtime().await;
    let adapter = WebAdapter::spawn(runtime.clone(), WebAdapterConfig::default())
        .await
        .expect("adapter spawn");

    let session = adapter
        .create_session(
            "tenant_a",
            CreateSessionRequest {
                artifact_id: "doc:approval".to_owned(),
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
    let mut events = adapter
        .subscribe_session_events("tenant_a", &session.session_id)
        .await
        .expect("subscribe events");

    adapter
        .create_turn(
            "tenant_a",
            &session.session_id,
            CreateTurnRequest {
                task: turn_task("need_approval"),
            },
        )
        .await
        .expect("create turn");

    let request = timeout(Duration::from_secs(2), approvals.recv())
        .await
        .expect("approval timeout")
        .expect("approval channel closed");
    assert_eq!(request.method, "item/fileChange/requestApproval");

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
        .expect("post approval");

    loop {
        let envelope = timeout(Duration::from_secs(2), events.recv())
            .await
            .expect("ack timeout")
            .expect("event channel closed");
        if envelope.method.as_deref() == Some("approval/ack") {
            assert_eq!(
                envelope.thread_id.as_deref(),
                Some(session.thread_id.as_str())
            );
            break;
        }
    }

    runtime.shutdown().await.expect("shutdown");
}
