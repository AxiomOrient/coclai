use super::*;

#[test]
fn restart_delay_stays_within_base_and_jitter_bounds() {
    for attempt in 0..8 {
        let delay = supervisor::compute_restart_delay(attempt, 10, 160);
        let delay_ms = delay.as_millis() as u64;
        let base_ms = (10u64.saturating_mul(1u64 << attempt)).min(160);
        let jitter_cap_ms = (base_ms / 10).min(1_000);

        assert!(delay_ms >= base_ms);
        assert!(delay_ms <= base_ms + jitter_cap_ms);
    }
}

#[tokio::test(flavor = "current_thread")]
async fn spawn_auto_initializes_runtime() {
    let runtime = spawn_mock_runtime().await;
    assert!(runtime.is_initialized());

    let value = runtime
        .call_raw("echo/test", json!({"k":"v"}))
        .await
        .expect("call");
    assert_eq!(value["echoMethod"], "echo/test");
    assert_eq!(value["params"]["k"], "v");

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn spawn_fails_fast_on_initialize_error_without_hanging() {
    let cfg = RuntimeConfig::new(python_initialize_error_process(), workspace_schema_guard());
    let result = timeout(Duration::from_secs(3), Runtime::spawn_local(cfg))
        .await
        .expect("spawn_local must not hang");

    let err = match result {
        Ok(_) => panic!("spawn_local must fail on initialize error"),
        Err(err) => err,
    };
    match err {
        RuntimeError::Internal(message) => {
            assert!(message.contains("initialize handshake failed"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[tokio::test(flavor = "current_thread")]
async fn spawn_rejects_zero_channel_capacities() {
    let mut cfg = RuntimeConfig::new(python_mock_process(), workspace_schema_guard());
    cfg.live_channel_capacity = 0;
    let err = match Runtime::spawn_local(cfg).await {
        Ok(_) => panic!("must reject zero live channel capacity"),
        Err(err) => err,
    };
    assert!(matches!(err, RuntimeError::InvalidConfig(_)));

    let mut cfg = RuntimeConfig::new(python_mock_process(), workspace_schema_guard());
    cfg.server_request_channel_capacity = 0;
    let err = match Runtime::spawn_local(cfg).await {
        Ok(_) => panic!("must reject zero server-request channel capacity"),
        Err(err) => err,
    };
    assert!(matches!(err, RuntimeError::InvalidConfig(_)));

    let mut cfg = RuntimeConfig::new(python_mock_process(), workspace_schema_guard());
    cfg.event_sink = Some(Arc::new(FailAfterSink::new(0)));
    cfg.event_sink_channel_capacity = 0;
    let err = match Runtime::spawn_local(cfg).await {
        Ok(_) => panic!("must reject zero event sink channel capacity"),
        Err(err) => err,
    };
    assert!(matches!(err, RuntimeError::InvalidConfig(_)));

    let mut cfg = RuntimeConfig::new(python_mock_process(), workspace_schema_guard());
    cfg.state_projection_limits.max_threads = 0;
    let err = match Runtime::spawn_local(cfg).await {
        Ok(_) => panic!("must reject zero state projection thread cap"),
        Err(err) => err,
    };
    assert!(matches!(err, RuntimeError::InvalidConfig(_)));

    let mut cfg = RuntimeConfig::new(python_mock_process(), workspace_schema_guard());
    cfg.rpc_response_timeout = Duration::ZERO;
    let err = match Runtime::spawn_local(cfg).await {
        Ok(_) => panic!("must reject zero rpc response timeout"),
        Err(err) => err,
    };
    assert!(matches!(err, RuntimeError::InvalidConfig(_)));
}

#[tokio::test(flavor = "current_thread")]
async fn matches_10k_request_response_pairs() {
    let runtime = spawn_mock_runtime().await;

    for i in 0..10_000u64 {
        let value = runtime
            .call_raw("echo/loop", json!({"index": i}))
            .await
            .expect("call");
        assert_eq!(value["echoMethod"], "echo/loop");
        assert_eq!(value["params"]["index"], i);
    }

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_restarts_after_forced_exit() {
    let runtime = spawn_runtime_with_supervisor(
        python_restartable_process(),
        RestartPolicy::OnCrash {
            max_restarts: 3,
            base_backoff_ms: 10,
            max_backoff_ms: 40,
        },
    )
    .await;

    let crash = runtime.call_raw("crash_now", json!({})).await;
    assert!(matches!(crash, Err(RpcError::TransportClosed)));

    let recovered = wait_for_recovery(&runtime).await;
    assert_eq!(recovered["echoMethod"], "echo/recovered");

    let snapshot = runtime.state_snapshot();
    match &snapshot.connection {
        ConnectionState::Running { generation } => assert!(*generation >= 1),
        other => panic!("unexpected connection state after recovery: {other:?}"),
    }

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn supervisor_transitions_dead_after_restart_limit_exceeded() {
    let runtime = spawn_runtime_with_supervisor(
        python_exit_on_initialized_process(),
        RestartPolicy::OnCrash {
            max_restarts: 1,
            base_backoff_ms: 10,
            max_backoff_ms: 20,
        },
    )
    .await;

    timeout(Duration::from_secs(3), async {
        loop {
            if runtime.state_snapshot().connection == ConnectionState::Dead {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("dead transition timeout");

    let err = runtime
        .call_raw("echo/dead", json!({}))
        .await
        .expect_err("must fail when dead");
    assert!(matches!(err, RpcError::InvalidRequest(_)));

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn pending_calls_resolve_transport_closed_on_child_exit() {
    let runtime =
        spawn_runtime_with_supervisor(python_hold_and_crash_process(), RestartPolicy::Never).await;

    let runtime_a = runtime.clone();
    let pending_a = tokio::spawn(async move { runtime_a.call_raw("hold", json!({"n":1})).await });

    let runtime_b = runtime.clone();
    let pending_b = tokio::spawn(async move { runtime_b.call_raw("hold", json!({"n":2})).await });

    sleep(Duration::from_millis(50)).await;
    let _ = runtime.notify_raw("crash_now", json!({})).await;

    let result_a = timeout(Duration::from_secs(2), pending_a)
        .await
        .expect("pending_a timeout")
        .expect("pending_a join");
    let result_b = timeout(Duration::from_secs(2), pending_b)
        .await
        .expect("pending_b timeout")
        .expect("pending_b join");

    assert!(matches!(result_a, Err(RpcError::TransportClosed)));
    assert!(matches!(result_b, Err(RpcError::TransportClosed)));

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn call_raw_returns_timeout_when_response_missing() {
    let mut cfg = RuntimeConfig::new(python_hold_and_crash_process(), workspace_schema_guard());
    cfg.rpc_response_timeout = Duration::from_millis(120);
    let runtime = Runtime::spawn_local(cfg).await.expect("runtime spawn");

    let started = Instant::now();
    let err = runtime
        .call_raw("hold", json!({"n":1}))
        .await
        .expect_err("hold call must timeout");
    assert!(matches!(err, RpcError::Timeout));
    assert!(
        started.elapsed() < Duration::from_millis(500),
        "rpc timeout exceeded expected bound: {:?}",
        started.elapsed()
    );

    let metrics = runtime.metrics_snapshot();
    assert_eq!(metrics.pending_rpc_count, 0);

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn call_validated_rejects_invalid_known_method_params() {
    let runtime = spawn_mock_runtime().await;

    let err = runtime
        .call_validated("turn/interrupt", json!({"threadId":"thr_only"}))
        .await
        .expect_err("missing turnId must fail");
    assert!(matches!(err, RpcError::InvalidRequest(_)));

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn call_validated_rejects_invalid_known_method_response_shape() {
    let runtime = spawn_mock_runtime().await;

    let err = runtime
        .call_validated("thread/start", json!({}))
        .await
        .expect_err("mock response does not include thread id");
    assert!(matches!(err, RpcError::InvalidRequest(_)));

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn notify_validated_rejects_invalid_known_method_params() {
    let runtime = spawn_mock_runtime().await;

    let err = runtime
        .notify_validated("turn/interrupt", json!({"threadId":"thr_only"}))
        .await
        .expect_err("missing turnId must fail");
    assert!(matches!(err, RuntimeError::InvalidConfig(_)));

    runtime.shutdown().await.expect("shutdown");
}

#[derive(Debug, Serialize)]
struct TurnInterruptNotifyMissingTurnId {
    #[serde(rename = "threadId")]
    thread_id: String,
}

#[derive(Debug, Serialize)]
struct TurnInterruptNotifyParams {
    #[serde(rename = "threadId")]
    thread_id: String,
    #[serde(rename = "turnId")]
    turn_id: String,
}

#[tokio::test(flavor = "current_thread")]
async fn notify_typed_validated_rejects_invalid_known_method_params() {
    let runtime = spawn_mock_runtime().await;

    let err = runtime
        .notify_typed_validated(
            "turn/interrupt",
            TurnInterruptNotifyMissingTurnId {
                thread_id: "thr_only".to_owned(),
            },
        )
        .await
        .expect_err("missing turnId must fail");
    assert!(matches!(err, RuntimeError::InvalidConfig(_)));

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn notify_typed_validated_accepts_valid_known_method_params() {
    let runtime = spawn_mock_runtime().await;

    runtime
        .notify_typed_validated(
            "turn/interrupt",
            TurnInterruptNotifyParams {
                thread_id: "thr_1".to_owned(),
                turn_id: "turn_1".to_owned(),
            },
        )
        .await
        .expect("valid turn/interrupt payload");

    runtime.shutdown().await.expect("shutdown");
}
