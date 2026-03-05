use super::*;

#[tokio::test(flavor = "current_thread")]
async fn sessions_turns_and_events_are_isolated() {
    let runtime = spawn_mock_runtime().await;
    let adapter = WebAdapter::spawn(runtime.clone(), WebAdapterConfig::default())
        .await
        .expect("adapter spawn");

    let session_a = adapter
        .create_session(
            "tenant_a",
            CreateSessionRequest {
                artifact_id: "doc:a".to_owned(),
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
                artifact_id: "doc:b".to_owned(),
                model: None,
                thread_id: None,
            },
        )
        .await
        .expect("create session b");
    assert_ne!(session_a.thread_id, session_b.thread_id);

    let mut events_a = adapter
        .subscribe_session_events("tenant_a", &session_a.session_id)
        .await
        .expect("events a");

    adapter
        .create_turn(
            "tenant_a",
            &session_a.session_id,
            CreateTurnRequest {
                task: turn_task("hello-a"),
            },
        )
        .await
        .expect("turn a");
    let completed_a = wait_turn_completed(&mut events_a, &session_a.thread_id).await;
    assert_eq!(
        completed_a.thread_id.as_deref(),
        Some(session_a.thread_id.as_str())
    );

    adapter
        .create_turn(
            "tenant_a",
            &session_b.session_id,
            CreateTurnRequest {
                task: turn_task("hello-b"),
            },
        )
        .await
        .expect("turn b");
    assert_no_thread_leak(
        &mut events_a,
        &session_b.thread_id,
        Duration::from_millis(250),
    )
    .await;

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn tenant_isolation_blocks_cross_access() {
    let runtime = spawn_mock_runtime().await;
    let adapter = WebAdapter::spawn(runtime.clone(), WebAdapterConfig::default())
        .await
        .expect("adapter spawn");

    let session = adapter
        .create_session(
            "tenant_a",
            CreateSessionRequest {
                artifact_id: "doc:a".to_owned(),
                model: None,
                thread_id: None,
            },
        )
        .await
        .expect("create session");

    let err = adapter
        .create_turn(
            "tenant_b",
            &session.session_id,
            CreateTurnRequest {
                task: turn_task("hello"),
            },
        )
        .await
        .expect_err("must block cross-tenant turn");
    assert_eq!(err, WebError::Forbidden);

    let err = adapter
        .subscribe_session_events("tenant_b", &session.session_id)
        .await
        .expect_err("must block cross-tenant event subscribe");
    assert_eq!(err, WebError::Forbidden);

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn create_session_rejects_untracked_thread_id() {
    let runtime = spawn_mock_runtime().await;
    let adapter = WebAdapter::spawn(runtime.clone(), WebAdapterConfig::default())
        .await
        .expect("adapter spawn");

    let err = adapter
        .create_session(
            "tenant_a",
            CreateSessionRequest {
                artifact_id: "doc:resume".to_owned(),
                model: None,
                thread_id: Some("thr_untracked".to_owned()),
            },
        )
        .await
        .expect_err("untracked thread id must be rejected");
    assert_eq!(err, WebError::Forbidden);

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn create_session_rejects_resume_thread_id_mismatch() {
    let (_live_tx, live_rx) = broadcast::channel::<Envelope>(8);
    let (_request_tx, request_rx) = tokio::sync::mpsc::channel::<ServerRequest>(8);
    let fake_state = Arc::new(Mutex::new(FakeWebAdapterState {
        start_thread_id: "thr_owned".to_owned(),
        resume_result_thread_id: Some("thr_unexpected".to_owned()),
        ..FakeWebAdapterState::default()
    }));
    let adapter: Arc<dyn WebPluginAdapter> = Arc::new(FakeWebAdapter {
        state: Arc::clone(&fake_state),
        streams: Arc::new(Mutex::new(Some(WebRuntimeStreams {
            request_rx,
            live_rx,
        }))),
    });
    let web = WebAdapter::spawn_with_adapter(adapter, WebAdapterConfig::default())
        .await
        .expect("spawn with fake adapter");

    let session = web
        .create_session(
            "tenant_a",
            CreateSessionRequest {
                artifact_id: "doc:resume-mismatch".to_owned(),
                model: None,
                thread_id: None,
            },
        )
        .await
        .expect("create seed session");
    assert_eq!(session.thread_id, "thr_owned");

    let err = web
        .create_session(
            "tenant_a",
            CreateSessionRequest {
                artifact_id: "doc:resume-mismatch".to_owned(),
                model: None,
                thread_id: Some("thr_owned".to_owned()),
            },
        )
        .await
        .expect_err("mismatched resume thread id must fail");
    match err {
        WebError::Internal(message) => {
            assert!(message.contains("thread/resume returned mismatched thread id"));
            assert!(message.contains("requested=thr_owned"));
            assert!(message.contains("actual=thr_unexpected"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[tokio::test(flavor = "current_thread")]
async fn close_session_removes_session_indexes() {
    let runtime = spawn_mock_runtime().await;
    let adapter = WebAdapter::spawn(runtime.clone(), WebAdapterConfig::default())
        .await
        .expect("adapter spawn");

    let session = adapter
        .create_session(
            "tenant_a",
            CreateSessionRequest {
                artifact_id: "doc:close".to_owned(),
                model: None,
                thread_id: None,
            },
        )
        .await
        .expect("create session");

    let closed = adapter
        .close_session("tenant_a", &session.session_id)
        .await
        .expect("close session");
    assert_eq!(closed.thread_id, session.thread_id);
    assert!(closed.archived);

    let err = adapter
        .subscribe_session_events("tenant_a", &session.session_id)
        .await
        .expect_err("session must be removed");
    assert_eq!(err, WebError::InvalidSession);

    let err = adapter
        .create_turn(
            "tenant_a",
            &session.session_id,
            CreateTurnRequest {
                task: turn_task("after-close"),
            },
        )
        .await
        .expect_err("closed session turn must fail");
    assert_eq!(err, WebError::InvalidSession);

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn close_session_rolls_back_when_archive_fails() {
    let runtime = spawn_mock_runtime().await;
    let adapter = WebAdapter::spawn(runtime.clone(), WebAdapterConfig::default())
        .await
        .expect("adapter spawn");

    let session = adapter
        .create_session(
            "tenant_a",
            CreateSessionRequest {
                artifact_id: "doc:close-fail".to_owned(),
                model: None,
                thread_id: None,
            },
        )
        .await
        .expect("create session");

    runtime.shutdown().await.expect("shutdown runtime first");

    let err = adapter
        .close_session("tenant_a", &session.session_id)
        .await
        .expect_err("close must fail when archive fails");
    match err {
        WebError::Internal(message) => {
            assert!(message.contains("thread/archive failed for session"));
        }
        other => panic!("unexpected error: {other:?}"),
    }

    let _events = adapter
        .subscribe_session_events("tenant_a", &session.session_id)
        .await
        .expect("session must remain active after rollback");

    let err = adapter
        .create_turn(
            "tenant_a",
            &session.session_id,
            CreateTurnRequest {
                task: turn_task("after-rollback"),
            },
        )
        .await
        .expect_err("runtime is down, but session index must still exist");
    assert_ne!(err, WebError::InvalidSession);
    assert_ne!(err, WebError::SessionClosing);
}

#[tokio::test(flavor = "current_thread")]
async fn close_session_can_retry_after_archive_failure() {
    let (_live_tx, live_rx) = broadcast::channel::<Envelope>(8);
    let (_request_tx, request_rx) = tokio::sync::mpsc::channel::<ServerRequest>(8);
    let fake_state = Arc::new(Mutex::new(FakeWebAdapterState {
        start_thread_id: "thr_close_retry".to_owned(),
        archive_failures_remaining: 1,
        ..FakeWebAdapterState::default()
    }));
    let adapter: Arc<dyn WebPluginAdapter> = Arc::new(FakeWebAdapter {
        state: Arc::clone(&fake_state),
        streams: Arc::new(Mutex::new(Some(WebRuntimeStreams {
            request_rx,
            live_rx,
        }))),
    });
    let web = WebAdapter::spawn_with_adapter(adapter, WebAdapterConfig::default())
        .await
        .expect("spawn with fake adapter");

    let session = web
        .create_session(
            "tenant_a",
            CreateSessionRequest {
                artifact_id: "doc:close-retry".to_owned(),
                model: None,
                thread_id: None,
            },
        )
        .await
        .expect("create session");

    let first = web
        .close_session("tenant_a", &session.session_id)
        .await
        .expect_err("first close must fail by injected archive error");
    assert!(matches!(first, WebError::Internal(_)));

    let closed = web
        .close_session("tenant_a", &session.session_id)
        .await
        .expect("second close must succeed");
    assert_eq!(closed.thread_id, "thr_close_retry");
    assert!(closed.archived);

    let err = web
        .subscribe_session_events("tenant_a", &session.session_id)
        .await
        .expect_err("session must be removed after successful retry");
    assert_eq!(err, WebError::InvalidSession);
}
