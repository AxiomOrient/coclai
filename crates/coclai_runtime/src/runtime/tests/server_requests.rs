use super::*;

#[tokio::test(flavor = "current_thread")]
async fn routes_server_request_notification_and_unknown() {
    let runtime = spawn_mock_runtime().await;
    let mut live_rx = runtime.subscribe_live();
    let mut server_request_rx = runtime
        .take_server_request_rx()
        .await
        .expect("take server request rx");

    let value = runtime.call_raw("probe", json!({})).await.expect("probe");
    assert_eq!(value["echoMethod"], "probe");

    let req = timeout(Duration::from_secs(2), server_request_rx.recv())
        .await
        .expect("server request timeout")
        .expect("server request closed");
    assert_eq!(req.method, "item/fileChange/requestApproval");
    assert!(!req.approval_id.is_empty());
    assert_eq!(req.params["itemId"], "item_1");

    let mut saw_notification = false;
    let mut saw_unknown = false;
    let mut saw_response = false;
    for _ in 0..8 {
        let envelope = timeout(Duration::from_secs(2), live_rx.recv())
            .await
            .expect("live timeout")
            .expect("live closed");
        if envelope.kind == MsgKind::Notification
            && envelope.method.as_deref() == Some("turn/started")
        {
            saw_notification = true;
        }
        if envelope.kind == MsgKind::Unknown {
            saw_unknown = true;
        }
        if envelope.kind == MsgKind::Response {
            saw_response = true;
        }
        if saw_notification && saw_unknown && saw_response {
            break;
        }
    }

    assert!(saw_notification);
    assert!(saw_unknown);
    assert!(saw_response);

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn take_server_request_rx_is_single_consumer() {
    let runtime = spawn_mock_runtime().await;
    let _first = runtime
        .take_server_request_rx()
        .await
        .expect("first take server request rx");

    let err = runtime
        .take_server_request_rx()
        .await
        .expect_err("second take server request rx must fail");
    assert_eq!(err, RuntimeError::ServerRequestReceiverTaken);

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn approval_response_roundtrip_ok() {
    let runtime = spawn_mock_runtime().await;
    let mut live_rx = runtime.subscribe_live();
    let mut server_request_rx = runtime
        .take_server_request_rx()
        .await
        .expect("take server request rx");

    runtime.call_raw("probe", json!({})).await.expect("probe");
    let req = timeout(Duration::from_secs(2), server_request_rx.recv())
        .await
        .expect("server request timeout")
        .expect("server request closed");

    runtime
        .respond_approval_ok(&req.approval_id, json!({"decision":"accept"}))
        .await
        .expect("respond approval");

    let mut saw_ack = false;
    for _ in 0..8 {
        let envelope = timeout(Duration::from_secs(2), live_rx.recv())
            .await
            .expect("live timeout")
            .expect("live closed");
        if envelope.kind == MsgKind::Notification
            && envelope.method.as_deref() == Some("approval/ack")
            && envelope.json["params"]["approvalRpcId"] == 777
        {
            assert_eq!(envelope.json["params"]["result"]["decision"], "accept");
            saw_ack = true;
            break;
        }
    }

    assert!(saw_ack);
    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn sink_failure_does_not_block_approval_pending_or_live_stream() {
    let sink_impl = Arc::new(FailAfterSink::new(0));
    let sink: Arc<dyn EventSink> = sink_impl.clone();
    let runtime = spawn_mock_runtime_with_sink(sink, 16).await;

    let mut live_rx = runtime.subscribe_live();
    let mut server_request_rx = runtime
        .take_server_request_rx()
        .await
        .expect("take server request rx");

    runtime.call_raw("probe", json!({})).await.expect("probe");
    let req = timeout(Duration::from_secs(2), server_request_rx.recv())
        .await
        .expect("server request timeout")
        .expect("server request closed");

    runtime
        .respond_approval_ok(&req.approval_id, json!({"decision":"accept"}))
        .await
        .expect("respond approval");

    let mut saw_ack = false;
    for _ in 0..12 {
        let envelope = timeout(Duration::from_secs(2), live_rx.recv())
            .await
            .expect("live timeout")
            .expect("live closed");
        if envelope.kind == MsgKind::Notification
            && envelope.method.as_deref() == Some("approval/ack")
            && envelope.json["params"]["approvalRpcId"] == 777
        {
            saw_ack = true;
            break;
        }
    }
    assert!(saw_ack, "live stream must continue even when sink fails");

    timeout(Duration::from_secs(2), async {
        loop {
            if sink_impl.failures() > 0 {
                break;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("sink failure not observed");

    let value = runtime
        .call_raw("echo/after_sink_failure", json!({"ok":true}))
        .await
        .expect("pending rpc path must continue");
    assert_eq!(value["echoMethod"], "echo/after_sink_failure");
    assert!(sink_impl.seen() >= 1);
    let metrics = runtime.metrics_snapshot();
    assert!(metrics.sink_write_count >= 1);
    assert!(metrics.sink_write_error_count >= 1);
    assert_eq!(metrics.pending_rpc_count, 0);
    assert_eq!(metrics.pending_server_request_count, 0);

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn metrics_snapshot_tracks_pending_and_broadcast_drop() {
    let runtime = spawn_mock_runtime().await;
    let mut server_request_rx = runtime
        .take_server_request_rx()
        .await
        .expect("take server request rx");

    runtime.call_raw("probe", json!({})).await.expect("probe");
    let req = timeout(Duration::from_secs(2), server_request_rx.recv())
        .await
        .expect("server request timeout")
        .expect("server request closed");
    runtime
        .respond_approval_ok(&req.approval_id, json!({"decision":"accept"}))
        .await
        .expect("respond approval");

    let metrics = runtime.metrics_snapshot();
    assert!(metrics.ingress_total >= 1);
    assert_eq!(metrics.pending_rpc_count, 0);
    assert_eq!(metrics.pending_server_request_count, 0);
    assert!(
        metrics.broadcast_send_failed >= 1,
        "no live subscribers should count as broadcast send failure"
    );

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn approval_payload_validation_failure_then_success() {
    let runtime = spawn_mock_runtime().await;
    let mut live_rx = runtime.subscribe_live();
    let mut server_request_rx = runtime
        .take_server_request_rx()
        .await
        .expect("take server request rx");

    runtime.call_raw("probe", json!({})).await.expect("probe");
    let req = timeout(Duration::from_secs(2), server_request_rx.recv())
        .await
        .expect("server request timeout")
        .expect("server request closed");

    let invalid = runtime
        .respond_approval_ok(&req.approval_id, json!({"unexpected":true}))
        .await;
    assert!(invalid.is_err(), "invalid payload must fail");

    runtime
        .respond_approval_ok(&req.approval_id, json!({"decision":"accept"}))
        .await
        .expect("respond approval");

    let mut saw_ack = false;
    for _ in 0..8 {
        let envelope = timeout(Duration::from_secs(2), live_rx.recv())
            .await
            .expect("live timeout")
            .expect("live closed");
        if envelope.kind == MsgKind::Notification
            && envelope.method.as_deref() == Some("approval/ack")
            && envelope.json["params"]["approvalRpcId"] == 777
        {
            saw_ack = true;
            break;
        }
    }
    assert!(saw_ack);

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn tool_request_user_input_roundtrip() {
    let runtime = spawn_mock_runtime().await;
    let mut live_rx = runtime.subscribe_live();
    let mut server_request_rx = runtime
        .take_server_request_rx()
        .await
        .expect("take server request rx");

    runtime
        .call_raw("probe_user_input", json!({}))
        .await
        .expect("probe_user_input");
    let req = timeout(Duration::from_secs(2), server_request_rx.recv())
        .await
        .expect("server request timeout")
        .expect("server request closed");
    assert_eq!(req.method, "item/tool/requestUserInput");

    runtime
        .respond_approval_ok(
            &req.approval_id,
            json!({
                "answers": {
                    "q1": {
                        "answers": ["alice"]
                    }
                }
            }),
        )
        .await
        .expect("respond user input");

    let mut saw_ack = false;
    for _ in 0..8 {
        let envelope = timeout(Duration::from_secs(2), live_rx.recv())
            .await
            .expect("live timeout")
            .expect("live closed");
        if envelope.kind == MsgKind::Notification
            && envelope.method.as_deref() == Some("approval/ack")
            && envelope.json["params"]["approvalRpcId"] == 780
        {
            assert_eq!(
                envelope.json["params"]["result"]["answers"]["q1"]["answers"][0],
                "alice"
            );
            saw_ack = true;
            break;
        }
    }
    assert!(saw_ack);

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn dynamic_tool_call_roundtrip() {
    let runtime = spawn_mock_runtime().await;
    let mut live_rx = runtime.subscribe_live();
    let mut server_request_rx = runtime
        .take_server_request_rx()
        .await
        .expect("take server request rx");

    runtime
        .call_raw("probe_dynamic_tool_call", json!({}))
        .await
        .expect("probe_dynamic_tool_call");
    let req = timeout(Duration::from_secs(2), server_request_rx.recv())
        .await
        .expect("server request timeout")
        .expect("server request closed");
    assert_eq!(req.method, "item/tool/call");

    runtime
        .respond_approval_ok(
            &req.approval_id,
            json!({
                "success": true,
                "contentItems": [{"type":"inputText","text":"done"}]
            }),
        )
        .await
        .expect("respond tool call");

    let mut saw_ack = false;
    for _ in 0..8 {
        let envelope = timeout(Duration::from_secs(2), live_rx.recv())
            .await
            .expect("live timeout")
            .expect("live closed");
        if envelope.kind == MsgKind::Notification
            && envelope.method.as_deref() == Some("approval/ack")
            && envelope.json["params"]["approvalRpcId"] == 781
        {
            assert_eq!(envelope.json["params"]["result"]["success"], true);
            assert_eq!(
                envelope.json["params"]["result"]["contentItems"][0]["text"],
                "done"
            );
            saw_ack = true;
            break;
        }
    }
    assert!(saw_ack);

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn auth_refresh_roundtrip() {
    let runtime = spawn_mock_runtime().await;
    let mut live_rx = runtime.subscribe_live();
    let mut server_request_rx = runtime
        .take_server_request_rx()
        .await
        .expect("take server request rx");

    runtime
        .call_raw("probe_auth_refresh", json!({}))
        .await
        .expect("probe_auth_refresh");
    let req = timeout(Duration::from_secs(2), server_request_rx.recv())
        .await
        .expect("server request timeout")
        .expect("server request closed");
    assert_eq!(req.method, "account/chatgptAuthTokens/refresh");

    runtime
        .respond_approval_ok(
            &req.approval_id,
            json!({
                "accessToken": "at_mock",
                "chatgptAccountId": "acct_1",
                "chatgptPlanType": null
            }),
        )
        .await
        .expect("respond auth refresh");

    let mut saw_ack = false;
    for _ in 0..8 {
        let envelope = timeout(Duration::from_secs(2), live_rx.recv())
            .await
            .expect("live timeout")
            .expect("live closed");
        if envelope.kind == MsgKind::Notification
            && envelope.method.as_deref() == Some("approval/ack")
            && envelope.json["params"]["approvalRpcId"] == 782
        {
            assert_eq!(envelope.json["params"]["result"]["accessToken"], "at_mock");
            assert_eq!(
                envelope.json["params"]["result"]["chatgptAccountId"],
                "acct_1"
            );
            saw_ack = true;
            break;
        }
    }
    assert!(saw_ack);

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn tool_request_user_input_payload_validation_rejects_missing_answers() {
    let runtime = spawn_mock_runtime().await;
    let mut server_request_rx = runtime
        .take_server_request_rx()
        .await
        .expect("take server request rx");

    runtime
        .call_raw("probe_user_input", json!({}))
        .await
        .expect("probe_user_input");
    let req = timeout(Duration::from_secs(2), server_request_rx.recv())
        .await
        .expect("server request timeout")
        .expect("server request closed");
    assert_eq!(req.method, "item/tool/requestUserInput");

    let invalid = runtime
        .respond_approval_ok(&req.approval_id, json!({"decision":"cancel"}))
        .await;
    assert!(invalid.is_err(), "missing answers object must fail");

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn dynamic_tool_call_payload_validation_rejects_missing_content_items() {
    let runtime = spawn_mock_runtime().await;
    let mut server_request_rx = runtime
        .take_server_request_rx()
        .await
        .expect("take server request rx");

    runtime
        .call_raw("probe_dynamic_tool_call", json!({}))
        .await
        .expect("probe_dynamic_tool_call");
    let req = timeout(Duration::from_secs(2), server_request_rx.recv())
        .await
        .expect("server request timeout")
        .expect("server request closed");
    assert_eq!(req.method, "item/tool/call");

    let invalid = runtime
        .respond_approval_ok(&req.approval_id, json!({"success":true}))
        .await;
    assert!(invalid.is_err(), "missing contentItems must fail");

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn auth_refresh_payload_validation_rejects_missing_access_token() {
    let runtime = spawn_mock_runtime().await;
    let mut server_request_rx = runtime
        .take_server_request_rx()
        .await
        .expect("take server request rx");

    runtime
        .call_raw("probe_auth_refresh", json!({}))
        .await
        .expect("probe_auth_refresh");
    let req = timeout(Duration::from_secs(2), server_request_rx.recv())
        .await
        .expect("server request timeout")
        .expect("server request closed");
    assert_eq!(req.method, "account/chatgptAuthTokens/refresh");

    let invalid = runtime
        .respond_approval_ok(
            &req.approval_id,
            json!({
                "chatgptAccountId": "acct_1",
                "chatgptPlanType": "plus"
            }),
        )
        .await;
    assert!(invalid.is_err(), "missing accessToken must fail");

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn server_request_with_string_id_roundtrip() {
    let runtime = spawn_mock_runtime().await;
    let mut live_rx = runtime.subscribe_live();
    let mut server_request_rx = runtime
        .take_server_request_rx()
        .await
        .expect("take server request rx");

    runtime
        .call_raw("probe_string_id", json!({}))
        .await
        .expect("probe_string_id");
    let req = timeout(Duration::from_secs(2), server_request_rx.recv())
        .await
        .expect("server request timeout")
        .expect("server request closed");
    assert_eq!(req.method, "item/fileChange/requestApproval");

    let snapshot = runtime.state_snapshot();
    assert!(
        snapshot.pending_server_requests.contains_key("s:req_str_1"),
        "state must index string request id"
    );

    runtime
        .respond_approval_ok(&req.approval_id, json!({"decision":"decline"}))
        .await
        .expect("respond approval");

    let mut saw_server_request_envelope = false;
    let mut saw_ack = false;
    for _ in 0..12 {
        let envelope = timeout(Duration::from_secs(2), live_rx.recv())
            .await
            .expect("live timeout")
            .expect("live closed");
        if envelope.kind == MsgKind::ServerRequest
            && envelope.method.as_deref() == Some("item/fileChange/requestApproval")
        {
            assert_eq!(
                envelope.rpc_id,
                Some(JsonRpcId::Text("req_str_1".to_owned()))
            );
            saw_server_request_envelope = true;
        }
        if envelope.kind == MsgKind::Notification
            && envelope.method.as_deref() == Some("approval/ack")
            && envelope.json["params"]["approvalRpcId"] == "req_str_1"
        {
            assert_eq!(envelope.json["params"]["result"]["decision"], "decline");
            saw_ack = true;
            if saw_server_request_envelope {
                break;
            }
        }
    }
    assert!(saw_server_request_envelope);
    assert!(saw_ack);

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn unknown_server_request_is_auto_declined() {
    let runtime = spawn_mock_runtime().await;
    let mut live_rx = runtime.subscribe_live();
    let mut server_request_rx = runtime
        .take_server_request_rx()
        .await
        .expect("take server request rx");

    runtime
        .call_raw("probe_unknown", json!({}))
        .await
        .expect("probe_unknown");

    let queued = timeout(Duration::from_millis(200), server_request_rx.recv()).await;
    assert!(queued.is_err(), "unknown request should not reach queue");

    let mut saw_ack = false;
    for _ in 0..8 {
        let envelope = timeout(Duration::from_secs(2), live_rx.recv())
            .await
            .expect("live timeout")
            .expect("live closed");
        if envelope.kind == MsgKind::Notification
            && envelope.method.as_deref() == Some("approval/ack")
            && envelope.json["params"]["approvalRpcId"] == 778
        {
            assert_eq!(envelope.json["params"]["result"]["decision"], "decline");
            saw_ack = true;
            break;
        }
    }

    assert!(saw_ack);
    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn timeout_policy_decline_replies_without_stall() {
    let runtime = spawn_mock_runtime_with_server_cfg(ServerRequestConfig {
        default_timeout_ms: 50,
        on_timeout: TimeoutAction::Decline,
        auto_decline_unknown: true,
    })
    .await;
    let mut live_rx = runtime.subscribe_live();
    let mut server_request_rx = runtime
        .take_server_request_rx()
        .await
        .expect("take server request rx");

    runtime
        .call_raw("probe_timeout", json!({}))
        .await
        .expect("probe_timeout");

    let req = timeout(Duration::from_secs(2), server_request_rx.recv())
        .await
        .expect("server request timeout")
        .expect("server request closed");
    assert_eq!(req.method, "item/fileChange/requestApproval");

    let mut saw_ack = false;
    for _ in 0..16 {
        let envelope = timeout(Duration::from_secs(2), live_rx.recv())
            .await
            .expect("live timeout")
            .expect("live closed");
        if envelope.kind == MsgKind::Notification
            && envelope.method.as_deref() == Some("approval/ack")
            && envelope.json["params"]["approvalRpcId"] == 779
        {
            assert_eq!(envelope.json["params"]["result"]["decision"], "decline");
            saw_ack = true;
            break;
        }
    }

    assert!(saw_ack);
    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn timeout_policy_decline_returns_empty_answers_for_user_input() {
    let runtime = spawn_mock_runtime_with_server_cfg(ServerRequestConfig {
        default_timeout_ms: 50,
        on_timeout: TimeoutAction::Decline,
        auto_decline_unknown: true,
    })
    .await;
    let mut live_rx = runtime.subscribe_live();
    let mut server_request_rx = runtime
        .take_server_request_rx()
        .await
        .expect("take server request rx");

    runtime
        .call_raw("probe_user_input", json!({}))
        .await
        .expect("probe_user_input");
    let req = timeout(Duration::from_secs(2), server_request_rx.recv())
        .await
        .expect("server request timeout")
        .expect("server request closed");
    assert_eq!(req.method, "item/tool/requestUserInput");

    let mut saw_ack = false;
    for _ in 0..16 {
        let envelope = timeout(Duration::from_secs(2), live_rx.recv())
            .await
            .expect("live timeout")
            .expect("live closed");
        if envelope.kind == MsgKind::Notification
            && envelope.method.as_deref() == Some("approval/ack")
            && envelope.json["params"]["approvalRpcId"] == 780
        {
            assert!(envelope.json["params"]["result"]["answers"].is_object());
            assert_eq!(
                envelope.json["params"]["result"]["answers"]
                    .as_object()
                    .expect("answers object")
                    .len(),
                0
            );
            saw_ack = true;
            break;
        }
    }

    assert!(saw_ack);
    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn timeout_policy_decline_returns_failure_payload_for_dynamic_tool_call() {
    let runtime = spawn_mock_runtime_with_server_cfg(ServerRequestConfig {
        default_timeout_ms: 50,
        on_timeout: TimeoutAction::Decline,
        auto_decline_unknown: true,
    })
    .await;
    let mut live_rx = runtime.subscribe_live();
    let mut server_request_rx = runtime
        .take_server_request_rx()
        .await
        .expect("take server request rx");

    runtime
        .call_raw("probe_dynamic_tool_call", json!({}))
        .await
        .expect("probe_dynamic_tool_call");
    let req = timeout(Duration::from_secs(2), server_request_rx.recv())
        .await
        .expect("server request timeout")
        .expect("server request closed");
    assert_eq!(req.method, "item/tool/call");

    let mut saw_ack = false;
    for _ in 0..16 {
        let envelope = timeout(Duration::from_secs(2), live_rx.recv())
            .await
            .expect("live timeout")
            .expect("live closed");
        if envelope.kind == MsgKind::Notification
            && envelope.method.as_deref() == Some("approval/ack")
            && envelope.json["params"]["approvalRpcId"] == 781
        {
            assert_eq!(envelope.json["params"]["result"]["success"], false);
            assert_eq!(envelope.json["params"]["result"]["contentItems"], json!([]));
            saw_ack = true;
            break;
        }
    }

    assert!(saw_ack);
    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn timeout_policy_decline_returns_error_for_auth_refresh() {
    let runtime = spawn_mock_runtime_with_server_cfg(ServerRequestConfig {
        default_timeout_ms: 50,
        on_timeout: TimeoutAction::Decline,
        auto_decline_unknown: true,
    })
    .await;
    let mut live_rx = runtime.subscribe_live();
    let mut server_request_rx = runtime
        .take_server_request_rx()
        .await
        .expect("take server request rx");

    runtime
        .call_raw("probe_auth_refresh", json!({}))
        .await
        .expect("probe_auth_refresh");
    let req = timeout(Duration::from_secs(2), server_request_rx.recv())
        .await
        .expect("server request timeout")
        .expect("server request closed");
    assert_eq!(req.method, "account/chatgptAuthTokens/refresh");

    let mut saw_ack = false;
    for _ in 0..16 {
        let envelope = timeout(Duration::from_secs(2), live_rx.recv())
            .await
            .expect("live timeout")
            .expect("live closed");
        if envelope.kind == MsgKind::Notification
            && envelope.method.as_deref() == Some("approval/ack")
            && envelope.json["params"]["approvalRpcId"] == 782
        {
            assert_eq!(envelope.json["params"]["error"]["code"], -32000);
            assert_eq!(
                envelope.json["params"]["error"]["data"]["method"],
                "account/chatgptAuthTokens/refresh"
            );
            saw_ack = true;
            break;
        }
    }

    assert!(saw_ack);
    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn closed_server_request_queue_resolves_immediately() {
    let runtime = spawn_mock_runtime_with_server_cfg(ServerRequestConfig {
        default_timeout_ms: 30_000,
        on_timeout: TimeoutAction::Decline,
        auto_decline_unknown: true,
    })
    .await;
    let mut live_rx = runtime.subscribe_live();

    let server_request_rx = runtime
        .take_server_request_rx()
        .await
        .expect("take server request rx");
    drop(server_request_rx);

    let started = std::time::Instant::now();
    runtime
        .call_raw("probe_timeout", json!({}))
        .await
        .expect("probe_timeout");

    let mut saw_ack = false;
    for _ in 0..16 {
        let envelope = timeout(Duration::from_secs(2), live_rx.recv())
            .await
            .expect("live timeout")
            .expect("live closed");
        if envelope.kind == MsgKind::Notification
            && envelope.method.as_deref() == Some("approval/ack")
            && envelope.json["params"]["approvalRpcId"] == 779
        {
            assert_eq!(envelope.json["params"]["result"]["decision"], "decline");
            saw_ack = true;
            break;
        }
    }

    assert!(saw_ack);
    assert!(started.elapsed() < Duration::from_secs(1));
    let snapshot = runtime.state_snapshot();
    assert!(snapshot.pending_server_requests.is_empty());
    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn call_raw_fails_when_not_initialized() {
    let runtime = spawn_mock_runtime().await;
    runtime.shutdown().await.expect("shutdown");

    let err = runtime
        .call_raw("echo/test", json!({}))
        .await
        .expect_err("must fail");
    assert!(matches!(err, RpcError::InvalidRequest(_)));
}
