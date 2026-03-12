use super::*;
use tokio::time::{sleep, Instant};

async fn spawn_fake_web_adapter_with_request_tx(
) -> (WebAdapter, tokio::sync::mpsc::Sender<ServerRequest>) {
    spawn_fake_web_adapter_with_request_tx_and_config(WebAdapterConfig::default()).await
}

async fn spawn_fake_web_adapter_with_request_tx_and_config(
    config: WebAdapterConfig,
) -> (WebAdapter, tokio::sync::mpsc::Sender<ServerRequest>) {
    let (_live_tx, live_rx) = broadcast::channel::<Envelope>(8);
    let (request_tx, request_rx) = tokio::sync::mpsc::channel::<ServerRequest>(8);
    let fake_state = Arc::new(Mutex::new(FakeWebAdapterState::default()));
    let adapter: Arc<dyn WebPluginAdapter> = Arc::new(FakeWebAdapter {
        state: fake_state,
        streams: Arc::new(Mutex::new(Some(WebRuntimeStreams {
            request_rx,
            live_rx,
        }))),
    });
    let web = WebAdapter::spawn_with_adapter(adapter, config)
        .await
        .expect("spawn with fake adapter");
    (web, request_tx)
}

async fn wait_route_miss_counts(web: &WebAdapter, expected: (u64, u64, u64)) {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let got = web.debug_server_request_route_miss_counts().await;
        if got == expected {
            return;
        }
        if Instant::now() >= deadline {
            panic!("route miss counters did not converge: expected={expected:?} got={got:?}");
        }
        sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test(flavor = "current_thread")]
async fn route_server_request_counts_missing_thread_id() {
    let (web, request_tx) = spawn_fake_web_adapter_with_request_tx().await;
    request_tx
        .send(ServerRequest {
            approval_id: "ap_missing_thread_id".to_owned(),
            method: "item/fileChange/requestApproval".to_owned(),
            params: json!({"turnId":"turn_1"}),
        })
        .await
        .expect("send request");

    wait_route_miss_counts(&web, (1, 0, 0)).await;
}

#[tokio::test(flavor = "current_thread")]
async fn route_server_request_counts_missing_session_mapping() {
    let (web, request_tx) = spawn_fake_web_adapter_with_request_tx().await;
    request_tx
        .send(ServerRequest {
            approval_id: "ap_missing_session".to_owned(),
            method: "item/fileChange/requestApproval".to_owned(),
            params: json!({"threadId":"thr_unmapped","turnId":"turn_1"}),
        })
        .await
        .expect("send request");

    wait_route_miss_counts(&web, (0, 1, 0)).await;
}

#[tokio::test(flavor = "current_thread")]
async fn route_server_request_counts_missing_approval_topic() {
    let (web, request_tx) = spawn_fake_web_adapter_with_request_tx().await;
    let session = web
        .create_session(
            "tenant_a",
            CreateSessionRequest {
                artifact_id: "doc:routing-miss-topic".to_owned(),
                model: None,
                thread_id: None,
            },
        )
        .await
        .expect("create session");
    web.debug_remove_approval_topic(&session.session_id).await;

    request_tx
        .send(ServerRequest {
            approval_id: "ap_missing_topic".to_owned(),
            method: "item/fileChange/requestApproval".to_owned(),
            params: json!({"threadId":session.thread_id,"turnId":"turn_1"}),
        })
        .await
        .expect("send request");

    wait_route_miss_counts(&web, (0, 0, 1)).await;
}

#[tokio::test(flavor = "current_thread")]
async fn queued_approvals_are_bounded_by_session_config_capacity() {
    let (web, request_tx) = spawn_fake_web_adapter_with_request_tx_and_config(WebAdapterConfig {
        session_event_channel_capacity: 8,
        session_approval_channel_capacity: 1,
    })
    .await;
    let session = web
        .create_session(
            "tenant_a",
            CreateSessionRequest {
                artifact_id: "doc:routing-bounded-queue".to_owned(),
                model: None,
                thread_id: None,
            },
        )
        .await
        .expect("create session");

    for approval_id in ["ap_1", "ap_2"] {
        request_tx
            .send(ServerRequest {
                approval_id: approval_id.to_owned(),
                method: "item/fileChange/requestApproval".to_owned(),
                params: json!({"threadId":session.thread_id,"turnId":"turn_1"}),
            })
            .await
            .expect("send request");
    }
    sleep(Duration::from_millis(50)).await;

    let mut approvals = web
        .subscribe_session_approvals("tenant_a", &session.session_id)
        .await
        .expect("subscribe approvals");

    let first = timeout(Duration::from_secs(2), approvals.recv())
        .await
        .expect("first approval timeout")
        .expect("approval channel closed");
    assert!(
        matches!(first.approval_id.as_str(), "ap_1" | "ap_2"),
        "received unexpected approval id: {}",
        first.approval_id
    );

    assert!(
        timeout(Duration::from_millis(200), approvals.recv())
            .await
            .is_err(),
        "queue should be capped at configured session capacity",
    );
}
