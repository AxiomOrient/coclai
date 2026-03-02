use super::*;

#[tokio::test(flavor = "current_thread")]
async fn state_snapshot_tracks_lifecycle_without_copy_on_read() {
    let runtime = spawn_mock_runtime().await;

    let before_a = runtime.state_snapshot();
    let before_b = runtime.state_snapshot();
    assert!(Arc::ptr_eq(&before_a, &before_b));
    assert_eq!(
        before_a.connection,
        ConnectionState::Running { generation: 0 }
    );

    runtime
        .call_raw("probe_state", json!({}))
        .await
        .expect("probe_state");

    let after = runtime.state_snapshot();
    let thread = after.threads.get("thr_state").expect("thread");
    assert_eq!(thread.active_turn, None);

    let turn = thread.turns.get("turn_state").expect("turn");
    assert_eq!(turn.status, crate::state::TurnStatus::Completed);
    let item = turn.items.get("item_state").expect("item");
    assert_eq!(item.text_accum, "hello");

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn state_snapshot_contains_pending_server_requests() {
    let runtime = spawn_mock_runtime_with_server_cfg(ServerRequestConfig {
        default_timeout_ms: 2_000,
        on_timeout: TimeoutAction::Decline,
        auto_decline_unknown: true,
    })
    .await;
    let mut server_request_rx = runtime
        .take_server_request_rx()
        .await
        .expect("take server request rx");

    runtime.call_raw("probe", json!({})).await.expect("probe");
    let req = timeout(Duration::from_secs(2), server_request_rx.recv())
        .await
        .expect("server request timeout")
        .expect("server request closed");

    let mid = runtime.state_snapshot();
    assert!(mid
        .pending_server_requests
        .values()
        .any(|v| v.approval_id == req.approval_id));

    runtime
        .respond_approval_ok(&req.approval_id, json!({"decision":"accept"}))
        .await
        .expect("respond approval");
    let after = runtime.state_snapshot();
    assert!(!after
        .pending_server_requests
        .values()
        .any(|v| v.approval_id == req.approval_id));

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn spawn_fails_when_metadata_missing_required_field() {
    let fixture = make_temp_schema_fixture(
        r#"{
  "schemaName":"app-server",
  "generatorCommand":"codex app-server generate-json-schema --out <DIR>",
  "sourceOfTruth":"active/json-schema"
}"#,
        &[],
        Some(""),
    );

    let cfg = RuntimeConfig::new(python_mock_process(), fixture.guard());
    let err = match Runtime::spawn_local(cfg).await {
        Ok(_) => panic!("spawn must fail"),
        Err(err) => err,
    };

    match err {
        RuntimeError::Internal(msg) => {
            assert!(msg.contains("metadata"), "unexpected error message: {msg}");
        }
        other => panic!("unexpected error type: {other:?}"),
    }
}

#[tokio::test(flavor = "current_thread")]
async fn spawn_fails_when_manifest_mismatches_schema_files() {
    let fixture = make_temp_schema_fixture(
        r#"{
  "schemaName":"app-server",
  "generatedAtUtc":"2026-01-01T00:00:00Z",
  "generatorCommand":"codex app-server generate-json-schema --out <DIR>",
  "sourceOfTruth":"active/json-schema"
}"#,
        &[("root.json", br#"{"type":"object"}"#)],
        Some("deadbeef  ./root.json"),
    );

    let cfg = RuntimeConfig::new(python_mock_process(), fixture.guard());
    let err = match Runtime::spawn_local(cfg).await {
        Ok(_) => panic!("spawn must fail"),
        Err(err) => err,
    };

    match err {
        RuntimeError::Internal(msg) => {
            assert!(msg.contains("manifest"), "unexpected error message: {msg}");
        }
        other => panic!("unexpected error type: {other:?}"),
    }
}
