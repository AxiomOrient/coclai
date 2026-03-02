use super::*;

#[tokio::test(flavor = "current_thread")]
async fn typed_thread_and_turn_roundtrip() {
    let runtime = spawn_mock_runtime().await;

    let thread = runtime
        .thread_start(ThreadStartParams {
            approval_policy: Some(ApprovalPolicy::Never),
            sandbox_policy: Some(SandboxPolicy::Preset(SandboxPreset::ReadOnly)),
            ..ThreadStartParams::default()
        })
        .await
        .expect("thread start");
    assert_eq!(thread.thread_id, "thr_typed");

    let turn = thread
        .turn_start(TurnStartParams {
            input: vec![InputItem::Text {
                text: "hi".to_owned(),
            }],
            ..TurnStartParams::default()
        })
        .await
        .expect("turn start");
    assert_eq!(turn.thread_id, "thr_typed");
    assert_eq!(turn.turn_id, "turn_typed");

    let steered = thread
        .turn_steer(
            &turn.turn_id,
            vec![InputItem::Text {
                text: "continue".to_owned(),
            }],
        )
        .await
        .expect("turn steer");
    assert_eq!(steered, "turn_typed");

    thread
        .turn_interrupt(&turn.turn_id)
        .await
        .expect("turn interrupt");

    let resumed = runtime
        .thread_resume("thr_old", ThreadStartParams::default())
        .await
        .expect("thread resume");
    assert_eq!(resumed.thread_id, "thr_old");

    let forked = runtime.thread_fork("thr_old").await.expect("thread fork");
    assert_eq!(forked.thread_id, "thr_forked");

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn thread_start_rejects_privileged_sandbox_without_explicit_opt_in() {
    let runtime = spawn_mock_runtime().await;

    let err = runtime
        .thread_start(ThreadStartParams {
            cwd: Some("/tmp".to_owned()),
            approval_policy: Some(ApprovalPolicy::OnRequest),
            sandbox_policy: Some(SandboxPolicy::Preset(SandboxPreset::DangerFullAccess)),
            privileged_escalation_approved: false,
            ..ThreadStartParams::default()
        })
        .await
        .expect_err("must reject privileged sandbox without explicit opt-in");
    assert!(matches!(err, crate::errors::RpcError::InvalidRequest(_)));

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn thread_start_rejects_privileged_sandbox_without_non_never_approval() {
    let runtime = spawn_mock_runtime().await;

    let err = runtime
        .thread_start(ThreadStartParams {
            cwd: Some("/tmp".to_owned()),
            approval_policy: Some(ApprovalPolicy::Never),
            sandbox_policy: Some(SandboxPolicy::Preset(SandboxPreset::DangerFullAccess)),
            privileged_escalation_approved: true,
            ..ThreadStartParams::default()
        })
        .await
        .expect_err("must reject privileged sandbox with never approval");
    assert!(matches!(err, crate::errors::RpcError::InvalidRequest(_)));

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn thread_start_rejects_privileged_sandbox_without_scope() {
    let runtime = spawn_mock_runtime().await;

    let err = runtime
        .thread_start(ThreadStartParams {
            cwd: None,
            approval_policy: Some(ApprovalPolicy::OnRequest),
            sandbox_policy: Some(SandboxPolicy::Preset(SandboxPreset::DangerFullAccess)),
            privileged_escalation_approved: true,
            ..ThreadStartParams::default()
        })
        .await
        .expect_err("must reject privileged sandbox without explicit scope");
    assert!(matches!(err, crate::errors::RpcError::InvalidRequest(_)));

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn turn_start_rejects_privileged_sandbox_without_explicit_opt_in() {
    let runtime = spawn_mock_runtime().await;
    let thread = runtime
        .thread_start(ThreadStartParams {
            approval_policy: Some(ApprovalPolicy::Never),
            sandbox_policy: Some(SandboxPolicy::Preset(SandboxPreset::ReadOnly)),
            ..ThreadStartParams::default()
        })
        .await
        .expect("thread start");

    let err = thread
        .turn_start(TurnStartParams {
            input: vec![InputItem::Text {
                text: "hi".to_owned(),
            }],
            cwd: Some("/tmp".to_owned()),
            approval_policy: Some(ApprovalPolicy::OnRequest),
            sandbox_policy: Some(SandboxPolicy::Preset(SandboxPreset::WorkspaceWrite {
                writable_roots: vec!["/tmp".to_owned()],
                network_access: false,
            })),
            privileged_escalation_approved: false,
            ..TurnStartParams::default()
        })
        .await
        .expect_err("must reject privileged turn without explicit opt-in");
    assert!(matches!(err, crate::errors::RpcError::InvalidRequest(_)));

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn thread_resume_requires_thread_id_in_response() {
    let runtime = spawn_thread_resume_missing_id_runtime().await;

    let err = runtime
        .thread_resume("thr_missing", ThreadStartParams::default())
        .await
        .expect_err("thread resume must fail without thread id in response");

    match err {
        RpcError::InvalidRequest(message) => {
            assert!(message.contains("thread/resume missing thread id in result"));
        }
        other => panic!("unexpected error: {other:?}"),
    }

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn thread_resume_requires_matching_thread_id() {
    let runtime = spawn_thread_resume_mismatched_id_runtime().await;

    let err = runtime
        .thread_resume("thr_expected", ThreadStartParams::default())
        .await
        .expect_err("thread resume must fail on mismatched thread id in response");

    match err {
        RpcError::InvalidRequest(message) => {
            assert!(message.contains("thread/resume returned mismatched thread id"));
            assert!(message.contains("requested=thr_expected"));
            assert!(message.contains("actual=thr_unexpected"));
        }
        other => panic!("unexpected error: {other:?}"),
    }

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn turn_start_rejects_empty_input() {
    let runtime = spawn_mock_runtime().await;
    let thread = runtime
        .thread_start(ThreadStartParams::default())
        .await
        .expect("thread start");

    let err = thread
        .turn_start(TurnStartParams::default())
        .await
        .expect_err("must fail");
    assert!(matches!(err, RpcError::InvalidRequest(_)));

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn runtime_archive_and_interrupt_wrappers_work() {
    let runtime = spawn_mock_runtime().await;

    runtime
        .turn_interrupt("thr_typed", "turn_typed")
        .await
        .expect("runtime turn interrupt");
    runtime
        .thread_archive("thr_typed")
        .await
        .expect("runtime thread archive");

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn runtime_thread_read_list_loaded_and_rollback_wrappers_work() {
    let runtime = spawn_mock_runtime().await;

    let read = runtime
        .thread_read(ThreadReadParams {
            thread_id: "thr_typed".to_owned(),
            include_turns: Some(true),
        })
        .await
        .expect("thread read");
    assert_eq!(read.thread.id, "thr_typed");
    assert_eq!(read.thread.source, "app-server");
    assert_eq!(read.thread.extra.get("turnsIncluded"), Some(&json!(true)));
    assert_eq!(read.thread.turns.len(), 1);
    assert_eq!(read.thread.turns[0].id, "turn_read_1");
    assert_eq!(read.thread.turns[0].status, ThreadTurnStatus::Completed);
    assert_eq!(read.thread.turns[0].items.len(), 1);
    assert_eq!(
        read.thread.turns[0].items[0].item_type,
        ThreadItemType::AgentMessage
    );
    match &read.thread.turns[0].items[0].payload {
        ThreadItemPayloadView::AgentMessage(data) => assert_eq!(data.text, "ok"),
        other => panic!("unexpected payload: {other:?}"),
    }

    let listed = runtime
        .thread_list(ThreadListParams {
            archived: Some(true),
            cursor: Some("cursor_a".to_owned()),
            limit: Some(5),
            model_providers: Some(vec!["openai".to_owned(), "anthropic".to_owned()]),
            sort_key: Some(ThreadListSortKey::UpdatedAt),
        })
        .await
        .expect("thread list");
    assert_eq!(listed.data.len(), 1);
    assert_eq!(listed.data[0].id, "thr_list");
    assert_eq!(listed.data[0].model_provider, "openai");
    assert_eq!(
        listed.data[0].extra.get("archivedFilter"),
        Some(&json!(true))
    );
    assert_eq!(
        listed.data[0].extra.get("sortKey"),
        Some(&json!("updated_at"))
    );
    assert_eq!(listed.data[0].extra.get("providerCount"), Some(&json!(2)));
    assert_eq!(listed.next_cursor.as_deref(), Some("cursor_a"));

    let loaded = runtime
        .thread_loaded_list(ThreadLoadedListParams {
            cursor: Some("loaded_cursor".to_owned()),
            limit: Some(1),
        })
        .await
        .expect("thread loaded list");
    assert_eq!(loaded.data, vec!["thr_loaded_1".to_owned()]);
    assert_eq!(loaded.next_cursor.as_deref(), Some("loaded_cursor"));

    let rollback = runtime
        .thread_rollback(ThreadRollbackParams {
            thread_id: "thr_typed".to_owned(),
            num_turns: 3,
        })
        .await
        .expect("thread rollback");
    assert_eq!(rollback.thread.id, "thr_typed");
    assert_eq!(
        rollback.thread.extra.get("rolledBackTurns"),
        Some(&json!(3))
    );
    assert_eq!(rollback.thread.turns.len(), 1);
    assert_eq!(rollback.thread.turns[0].status, ThreadTurnStatus::Failed);
    assert_eq!(
        rollback.thread.turns[0].items[0].item_type,
        ThreadItemType::CommandExecution
    );
    match &rollback.thread.turns[0].items[0].payload {
        ThreadItemPayloadView::CommandExecution(data) => {
            assert_eq!(data.command, "false");
            assert_eq!(data.status, "failed");
        }
        other => panic!("unexpected payload: {other:?}"),
    }

    runtime.shutdown().await.expect("shutdown");
}
