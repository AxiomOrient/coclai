use super::*;

#[tokio::test(flavor = "current_thread")]
async fn spawn_with_adapter_rejects_incompatible_contract_version() {
    let adapter: Arc<dyn WebPluginAdapter> = Arc::new(IncompatibleWebAdapter);
    let err = match WebAdapter::spawn_with_adapter(adapter, WebAdapterConfig::default()).await {
        Ok(_) => panic!("must reject mismatch"),
        Err(err) => err,
    };
    assert_eq!(
        err,
        WebError::IncompatibleContract {
            expected_major: 1,
            expected_minor: 0,
            actual_major: 2,
            actual_minor: 0,
        }
    );
}

#[tokio::test(flavor = "current_thread")]
async fn spawn_with_adapter_accepts_compatible_minor_contract_version() {
    let adapter: Arc<dyn WebPluginAdapter> = Arc::new(CompatibleMinorWebAdapter);
    let web = WebAdapter::spawn_with_adapter(adapter, WebAdapterConfig::default())
        .await
        .expect("minor version must remain compatible");
    drop(web);
}

#[tokio::test(flavor = "current_thread")]
async fn web_adapter_uses_plugin_boundary_without_runtime_dependency() {
    let (_live_tx, live_rx) = broadcast::channel::<Envelope>(8);
    let (_request_tx, request_rx) = tokio::sync::mpsc::channel::<ServerRequest>(8);
    let state = Arc::new(Mutex::new(FakeWebAdapterState::default()));
    let adapter: Arc<dyn WebPluginAdapter> = Arc::new(FakeWebAdapter {
        state: Arc::clone(&state),
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
                artifact_id: "doc:web-adapter".to_owned(),
                model: Some("gpt-fake".to_owned()),
                thread_id: None,
            },
        )
        .await
        .expect("create session");
    assert_eq!(session.thread_id, "thr_fake_web");

    let turn = web
        .create_turn(
            "tenant_a",
            &session.session_id,
            CreateTurnRequest {
                task: turn_task("hello"),
            },
        )
        .await
        .expect("create turn");
    assert_eq!(turn.turn_id, "turn_fake_web");

    let closed = web
        .close_session("tenant_a", &session.session_id)
        .await
        .expect("close session");
    assert_eq!(closed.thread_id, "thr_fake_web");
    assert!(closed.archived);

    let state = state.lock().expect("fake adapter state lock");
    assert_eq!(state.take_stream_calls, 1);
    assert_eq!(state.start_calls, 1);
    assert_eq!(state.turn_start_calls.len(), 1);
    assert_eq!(state.turn_start_calls[0]["threadId"], "thr_fake_web");
    assert_eq!(state.archive_calls, vec!["thr_fake_web".to_owned()]);
}

#[tokio::test(flavor = "current_thread")]
async fn dropping_non_last_clone_keeps_background_routing_alive() {
    let (live_tx, live_rx) = broadcast::channel::<Envelope>(8);
    let (_request_tx, request_rx) = tokio::sync::mpsc::channel::<ServerRequest>(8);
    let state = Arc::new(Mutex::new(FakeWebAdapterState::default()));
    let adapter: Arc<dyn WebPluginAdapter> = Arc::new(FakeWebAdapter {
        state: Arc::clone(&state),
        streams: Arc::new(Mutex::new(Some(WebRuntimeStreams {
            request_rx,
            live_rx,
        }))),
    });

    let web = WebAdapter::spawn_with_adapter(adapter, WebAdapterConfig::default())
        .await
        .expect("spawn with fake adapter");
    let dropped_clone = web.clone();
    drop(dropped_clone);

    let session = web
        .create_session(
            "tenant_a",
            CreateSessionRequest {
                artifact_id: "doc:clone-drop".to_owned(),
                model: None,
                thread_id: None,
            },
        )
        .await
        .expect("create session");
    let mut events = web
        .subscribe_session_events("tenant_a", &session.session_id)
        .await
        .expect("subscribe events");

    let envelope = Envelope {
        seq: 1,
        ts_millis: 0,
        direction: Direction::Inbound,
        kind: MsgKind::Notification,
        rpc_id: None,
        method: Some(Arc::<str>::from("turn/completed")),
        thread_id: Some(Arc::<str>::from(session.thread_id.clone())),
        turn_id: Some(Arc::<str>::from("turn_clone_drop")),
        item_id: None,
        json: Arc::new(json!({
            "method": "turn/completed",
            "params": {
                "threadId": session.thread_id.clone(),
                "turnId": "turn_clone_drop"
            }
        })),
    };
    live_tx.send(envelope).expect("send live event");

    let routed = timeout(Duration::from_secs(2), events.recv())
        .await
        .expect("event timeout")
        .expect("event channel closed");
    assert_eq!(
        routed.thread_id.as_deref(),
        Some(session.thread_id.as_str())
    );
    assert_eq!(routed.method.as_deref(), Some("turn/completed"));
}

#[tokio::test(flavor = "current_thread")]
async fn spawn_rejects_zero_capacity_config() {
    let runtime = spawn_mock_runtime().await;

    let err = match WebAdapter::spawn(
        runtime.clone(),
        WebAdapterConfig {
            session_event_channel_capacity: 0,
            session_approval_channel_capacity: 128,
        },
    )
    .await
    {
        Ok(_) => panic!("must reject zero event capacity"),
        Err(err) => err,
    };
    assert!(matches!(err, WebError::InvalidConfig(_)));

    let err = match WebAdapter::spawn(
        runtime.clone(),
        WebAdapterConfig {
            session_event_channel_capacity: 128,
            session_approval_channel_capacity: 0,
        },
    )
    .await
    {
        Ok(_) => panic!("must reject zero approval capacity"),
        Err(err) => err,
    };
    assert!(matches!(err, WebError::InvalidConfig(_)));

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn spawn_rejects_second_adapter_on_same_runtime() {
    let runtime = spawn_mock_runtime().await;
    let _adapter = WebAdapter::spawn(runtime.clone(), WebAdapterConfig::default())
        .await
        .expect("first adapter spawn");

    let err = match WebAdapter::spawn(runtime.clone(), WebAdapterConfig::default()).await {
        Ok(_) => panic!("second adapter on same runtime must fail"),
        Err(err) => err,
    };
    assert_eq!(err, WebError::AlreadyBound);

    runtime.shutdown().await.expect("shutdown");
}
