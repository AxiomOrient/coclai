use super::*;
use crate::runtime::{
    CommandExecParams, CommandExecResizeParams, CommandExecTerminalSize,
    CommandExecTerminateParams, CommandExecWriteParams, SkillScope, SkillsListParams,
};

#[tokio::test(flavor = "current_thread")]
async fn request_json_thread_start_returns_thread_id() {
    let app = connect_real_appserver().await;

    let thread_id = start_thread(&app).await;
    assert!(!thread_id.is_empty());

    archive_thread_best_effort(&app, &thread_id).await;
    app.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn request_typed_thread_read_returns_started_thread() {
    let app = connect_real_appserver().await;

    let thread_id = start_thread(&app).await;
    let read: ThreadReadResponse = app
        .request_typed(
            methods::THREAD_READ,
            ThreadReadParams {
                thread_id: thread_id.clone(),
                include_turns: Some(false),
            },
        )
        .await
        .expect("typed thread/read");
    assert_eq!(read.thread.id, thread_id);

    archive_thread_best_effort(&app, &read.thread.id).await;
    app.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn skills_list_wrapper_returns_inventory() {
    let app = connect_real_appserver().await;

    let listed = app
        .skills_list(SkillsListParams {
            cwds: vec!["/repo".to_owned()],
            force_reload: true,
            per_cwd_extra_user_roots: None,
        })
        .await
        .expect("skills/list");

    assert_eq!(listed.data.len(), 1);
    assert_eq!(listed.data[0].cwd, "/repo");
    assert_eq!(listed.data[0].skills[0].name, "skill-creator");
    assert_eq!(listed.data[0].skills[0].scope, SkillScope::Repo);

    app.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn command_exec_helpers_cover_buffered_and_follow_up_paths() {
    let app = connect_real_appserver().await;

    let buffered = app
        .command_exec(CommandExecParams {
            command: vec!["echo".to_owned(), "hi".to_owned()],
            cwd: Some("/repo".to_owned()),
            ..CommandExecParams::default()
        })
        .await
        .expect("buffered command exec");
    assert_eq!(buffered.exit_code, 0);
    assert_eq!(buffered.stdout, "buffered-stdout");
    assert_eq!(buffered.stderr, "buffered-stderr");

    app.command_exec_write(CommandExecWriteParams {
        process_id: "proc-1".to_owned(),
        delta_base64: Some("aGVsbG8=".to_owned()),
        close_stdin: false,
    })
    .await
    .expect("command exec write");

    app.command_exec_resize(CommandExecResizeParams {
        process_id: "proc-1".to_owned(),
        size: CommandExecTerminalSize {
            rows: 40,
            cols: 120,
        },
    })
    .await
    .expect("command exec resize");

    app.command_exec_terminate(CommandExecTerminateParams {
        process_id: "proc-1".to_owned(),
    })
    .await
    .expect("command exec terminate");

    app.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn request_json_rejects_invalid_known_params_before_send() {
    let app = connect_real_appserver().await;

    let err = app
        .request_json(methods::TURN_INTERRUPT, json!({"threadId":"thr"}))
        .await
        .expect_err("missing turnId must fail validation");
    assert!(matches!(err, RpcError::InvalidRequest(_)));

    app.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn notify_json_rejects_invalid_known_params_before_send() {
    let app = connect_real_appserver().await;

    let err = app
        .notify_json(methods::TURN_INTERRUPT, json!({"threadId":"thr"}))
        .await
        .expect_err("missing turnId must fail validation");
    assert!(matches!(err, RuntimeError::InvalidConfig(_)));

    app.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn request_json_rejects_empty_method_name_before_send() {
    let app = connect_real_appserver().await;

    let err = app
        .request_json("", json!({}))
        .await
        .expect_err("empty method name must fail request validation");
    assert!(matches!(err, RpcError::InvalidRequest(_)));

    app.shutdown().await.expect("shutdown");
}
