use super::*;

#[tokio::test(flavor = "current_thread")]
async fn run_prompt_returns_assistant_text() {
    let runtime = spawn_run_prompt_runtime().await;
    let result = runtime
        .run_prompt(PromptRunParams {
            cwd: "/tmp".to_owned(),
            prompt: "say ok".to_owned(),
            model: Some("gpt-5-codex".to_owned()),
            effort: Some(ReasoningEffort::High),
            approval_policy: ApprovalPolicy::Never,
            sandbox_policy: SandboxPolicy::Preset(SandboxPreset::ReadOnly),
            privileged_escalation_approved: false,
            attachments: vec![],
            timeout: Duration::from_secs(2),
        })
        .await
        .expect("run prompt");

    assert_eq!(result.thread_id, "thr_prompt");
    assert_eq!(result.turn_id, "turn_prompt");
    assert_eq!(result.assistant_text, "ok-from-run-prompt");

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn run_prompt_simple_returns_assistant_text() {
    let runtime = spawn_run_prompt_runtime().await;
    let result = runtime
        .run_prompt_simple("/tmp", "say ok")
        .await
        .expect("run prompt simple");

    assert_eq!(result.thread_id, "thr_prompt");
    assert_eq!(result.turn_id, "turn_prompt");
    assert_eq!(result.assistant_text, "ok-from-run-prompt");

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn run_prompt_in_thread_reuses_existing_thread_id() {
    let runtime = spawn_run_prompt_runtime().await;
    let result = runtime
        .run_prompt_in_thread(
            "thr_existing",
            PromptRunParams::new("/tmp", "continue conversation"),
        )
        .await
        .expect("run prompt in thread");

    assert_eq!(result.thread_id, "thr_existing");
    assert_eq!(result.turn_id, "turn_prompt");
    assert_eq!(result.assistant_text, "ok-from-run-prompt");

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn run_prompt_hook_order_is_pre_then_post() {
    let events = Arc::new(Mutex::new(Vec::<String>::new()));
    let hooks = RuntimeHookConfig::new()
        .with_pre_hook(Arc::new(RecordingPreHook {
            name: "pre_recorder",
            events: events.clone(),
            fail_phase: None,
        }))
        .with_post_hook(Arc::new(RecordingPostHook {
            name: "post_recorder",
            events: events.clone(),
            fail_phase: None,
        }));
    let runtime = spawn_run_prompt_runtime_with_hooks(hooks).await;

    let result = runtime
        .run_prompt(PromptRunParams::new("/tmp", "say ok"))
        .await
        .expect("run prompt");

    assert_eq!(result.thread_id, "thr_prompt");
    assert_eq!(result.turn_id, "turn_prompt");
    assert_eq!(result.assistant_text, "ok-from-run-prompt");
    assert!(
        runtime.hook_report_snapshot().is_clean(),
        "no hook issue expected"
    );
    assert_eq!(
        events.lock().expect("events lock").as_slice(),
        &[
            "pre:PreRun".to_owned(),
            "pre:PreTurn".to_owned(),
            "post:PostTurn".to_owned(),
            "post:PostRun".to_owned(),
        ]
    );

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn run_prompt_hook_failure_is_fail_open_with_report() {
    let events = Arc::new(Mutex::new(Vec::<String>::new()));
    let hooks = RuntimeHookConfig::new()
        .with_pre_hook(Arc::new(RecordingPreHook {
            name: "pre_fail",
            events: events.clone(),
            fail_phase: Some(HookPhase::PreRun),
        }))
        .with_post_hook(Arc::new(RecordingPostHook {
            name: "post_fail",
            events: events.clone(),
            fail_phase: Some(HookPhase::PostRun),
        }));
    let runtime = spawn_run_prompt_runtime_with_hooks(hooks).await;

    let result = runtime
        .run_prompt(PromptRunParams::new("/tmp", "say ok"))
        .await
        .expect("run prompt must continue despite hook failures");

    assert_eq!(result.assistant_text, "ok-from-run-prompt");
    let report = runtime.hook_report_snapshot();
    assert_eq!(report.issues.len(), 2);
    assert_eq!(report.issues[0].hook_name, "pre_fail");
    assert_eq!(report.issues[0].phase, HookPhase::PreRun);
    assert_eq!(report.issues[1].hook_name, "post_fail");
    assert_eq!(report.issues[1].phase, HookPhase::PostRun);
    assert_eq!(
        events.lock().expect("events lock").as_slice(),
        &[
            "pre:PreRun".to_owned(),
            "pre:PreTurn".to_owned(),
            "post:PostTurn".to_owned(),
            "post:PostRun".to_owned(),
        ]
    );

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn thread_start_and_resume_emit_session_hook_phases() {
    let events = Arc::new(Mutex::new(Vec::<String>::new()));
    let hooks = RuntimeHookConfig::new()
        .with_pre_hook(Arc::new(RecordingPreHook {
            name: "pre_session",
            events: events.clone(),
            fail_phase: None,
        }))
        .with_post_hook(Arc::new(RecordingPostHook {
            name: "post_session",
            events: events.clone(),
            fail_phase: None,
        }));
    let cfg =
        RuntimeConfig::new(python_api_mock_process(), workspace_schema_guard()).with_hooks(hooks);
    let runtime = Runtime::spawn_local(cfg).await.expect("spawn runtime");

    let started = runtime
        .thread_start(ThreadStartParams::default())
        .await
        .expect("thread start");
    assert_eq!(started.thread_id, "thr_typed");

    let resumed = runtime
        .thread_resume("thr_existing", ThreadStartParams::default())
        .await
        .expect("thread resume");
    assert_eq!(resumed.thread_id, "thr_existing");

    assert!(
        runtime.hook_report_snapshot().is_clean(),
        "session hooks should not report issue"
    );
    assert_eq!(
        events.lock().expect("events lock").as_slice(),
        &[
            "pre:PreSessionStart".to_owned(),
            "post:PostSessionStart".to_owned(),
            "pre:PreSessionStart".to_owned(),
            "post:PostSessionStart".to_owned(),
        ]
    );

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn run_prompt_applies_pre_mutations_for_prompt_model_attachment_and_metadata() {
    let existing_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../README.md")
        .to_string_lossy()
        .to_string();
    let patches = vec![
        (
            HookPhase::PreRun,
            coclai_plugin_core::HookPatch {
                prompt_override: Some("patched-in-pre-run".to_owned()),
                model_override: Some("model-pre-run".to_owned()),
                add_attachments: vec![coclai_plugin_core::HookAttachment::ImageUrl {
                    url: "https://example.com/x.png".to_owned(),
                }],
                metadata_delta: json!({"from_pre_run": true}),
            },
        ),
        (
            HookPhase::PreTurn,
            coclai_plugin_core::HookPatch {
                prompt_override: Some("patched-in-pre-turn".to_owned()),
                model_override: Some("model-pre-turn".to_owned()),
                add_attachments: vec![coclai_plugin_core::HookAttachment::Skill {
                    name: "probe".to_owned(),
                    path: existing_path.clone(),
                }],
                metadata_delta: json!({"from_pre_turn": 1}),
            },
        ),
    ];

    let metadata_events = Arc::new(Mutex::new(Vec::<(HookPhase, Value)>::new()));
    let hooks = RuntimeHookConfig::new()
        .with_pre_hook(Arc::new(PhasePatchPreHook {
            name: "phase_patch",
            patches,
        }))
        .with_post_hook(Arc::new(MetadataCapturePostHook {
            name: "metadata_capture",
            metadata: metadata_events.clone(),
        }));
    let runtime = spawn_run_prompt_mutation_probe_runtime(hooks).await;

    let result = runtime
        .run_prompt(PromptRunParams::new("/tmp", "original prompt"))
        .await
        .expect("run prompt");
    let payload: Value =
        serde_json::from_str(&result.assistant_text).expect("decode probe payload");
    assert_eq!(payload["threadModel"], json!("model-pre-run"));
    assert_eq!(payload["turnModel"], json!("model-pre-turn"));
    assert_eq!(payload["text"], json!("patched-in-pre-turn"));
    assert_eq!(payload["itemTypes"], json!(["text", "image", "skill"]),);

    let post_turn_metadata = {
        let captured = metadata_events.lock().expect("metadata lock");
        captured
            .iter()
            .find(|(phase, _)| *phase == HookPhase::PostTurn)
            .map(|(_, metadata)| metadata.clone())
            .expect("post-turn metadata")
    };
    assert_eq!(post_turn_metadata["from_pre_run"], json!(true));
    assert_eq!(post_turn_metadata["from_pre_turn"], json!(1));

    assert!(
        runtime.hook_report_snapshot().is_clean(),
        "valid mutations should not produce issues"
    );
    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn run_prompt_ignores_invalid_hook_attachment_with_fail_open() {
    let patches = vec![(
        HookPhase::PreTurn,
        coclai_plugin_core::HookPatch {
            prompt_override: None,
            model_override: None,
            add_attachments: vec![coclai_plugin_core::HookAttachment::LocalImage {
                path: "definitely_missing_image_for_hook_test.png".to_owned(),
            }],
            metadata_delta: Value::Null,
        },
    )];
    let hooks = RuntimeHookConfig::new().with_pre_hook(Arc::new(PhasePatchPreHook {
        name: "bad_attachment_patch",
        patches,
    }));
    let runtime = spawn_run_prompt_mutation_probe_runtime(hooks).await;

    let result = runtime
        .run_prompt(PromptRunParams::new("/tmp", "prompt"))
        .await
        .expect("main run should continue");
    let payload: Value =
        serde_json::from_str(&result.assistant_text).expect("decode probe payload");
    assert_eq!(payload["itemTypes"], json!(["text"]));
    let report = runtime.hook_report_snapshot();
    assert_eq!(report.issues.len(), 1);
    assert_eq!(report.issues[0].hook_name, "bad_attachment_patch");
    assert_eq!(report.issues[0].class, HookIssueClass::Validation);

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn pre_session_mutation_restricts_prompt_and_attachments_but_allows_model_and_metadata() {
    let patches = vec![(
        HookPhase::PreSessionStart,
        coclai_plugin_core::HookPatch {
            prompt_override: Some("not-allowed".to_owned()),
            model_override: Some("model-from-session-hook".to_owned()),
            add_attachments: vec![coclai_plugin_core::HookAttachment::ImageUrl {
                url: "https://example.com/ignored.png".to_owned(),
            }],
            metadata_delta: json!({"session_key": "session_value"}),
        },
    )];
    let metadata_events = Arc::new(Mutex::new(Vec::<(HookPhase, Value)>::new()));
    let hooks = RuntimeHookConfig::new()
        .with_pre_hook(Arc::new(PhasePatchPreHook {
            name: "session_patch",
            patches,
        }))
        .with_post_hook(Arc::new(MetadataCapturePostHook {
            name: "session_metadata_capture",
            metadata: metadata_events.clone(),
        }));
    let cfg = RuntimeConfig::new(
        python_session_mutation_probe_process(),
        workspace_schema_guard(),
    )
    .with_hooks(hooks);
    let runtime = Runtime::spawn_local(cfg).await.expect("spawn runtime");

    let thread = runtime
        .thread_start(ThreadStartParams::default())
        .await
        .expect("thread start");
    assert_eq!(thread.thread_id, "thr_model-from-session-hook");

    let report = runtime.hook_report_snapshot();
    assert_eq!(report.issues.len(), 2);
    assert!(report
        .issues
        .iter()
        .all(|issue| issue.class == HookIssueClass::Validation));
    assert_eq!(report.issues[0].phase, HookPhase::PreSessionStart);
    assert_eq!(report.issues[1].phase, HookPhase::PreSessionStart);

    let post_session_metadata = {
        let captured = metadata_events.lock().expect("metadata lock");
        captured
            .iter()
            .find(|(phase, _)| *phase == HookPhase::PostSessionStart)
            .map(|(_, metadata)| metadata.clone())
            .expect("post-session metadata")
    };
    assert_eq!(post_session_metadata["session_key"], json!("session_value"));

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn run_prompt_ignores_cross_thread_events_for_same_turn_id() {
    let runtime = spawn_run_prompt_cross_thread_noise_runtime().await;
    let result = runtime
        .run_prompt(PromptRunParams::new("/tmp", "say ok"))
        .await
        .expect("run prompt");

    assert_eq!(result.thread_id, "thr_prompt");
    assert_eq!(result.turn_id, "turn_prompt");
    assert_eq!(result.assistant_text, "ok-from-run-prompt");

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn run_prompt_simple_sends_default_effort() {
    let runtime = spawn_run_prompt_effort_probe_runtime().await;
    let result = runtime
        .run_prompt_simple("/tmp", "probe effort")
        .await
        .expect("run prompt simple");

    assert_eq!(result.assistant_text, "medium");

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn run_prompt_preserves_explicit_effort() {
    let runtime = spawn_run_prompt_effort_probe_runtime().await;
    let result = runtime
        .run_prompt(PromptRunParams {
            cwd: "/tmp".to_owned(),
            prompt: "probe effort".to_owned(),
            model: Some("gpt-5-codex".to_owned()),
            effort: Some(ReasoningEffort::High),
            approval_policy: ApprovalPolicy::Never,
            sandbox_policy: SandboxPolicy::Preset(SandboxPreset::ReadOnly),
            privileged_escalation_approved: false,
            attachments: vec![],
            timeout: Duration::from_secs(2),
        })
        .await
        .expect("run prompt");

    assert_eq!(result.assistant_text, "high");

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn run_prompt_surfaces_turn_error_when_text_is_empty() {
    let runtime = spawn_run_prompt_error_runtime().await;
    let err = runtime
        .run_prompt(PromptRunParams {
            cwd: "/tmp".to_owned(),
            prompt: "say ok".to_owned(),
            model: Some("gpt-5-codex".to_owned()),
            effort: Some(ReasoningEffort::High),
            approval_policy: ApprovalPolicy::Never,
            sandbox_policy: SandboxPolicy::Preset(SandboxPreset::ReadOnly),
            privileged_escalation_approved: false,
            attachments: vec![],
            timeout: Duration::from_secs(2),
        })
        .await
        .expect_err("run prompt must fail");

    match err {
        PromptRunError::TurnCompletedWithoutAssistantText(failure) => {
            assert_eq!(
                failure.terminal_state,
                PromptTurnTerminalState::CompletedWithoutAssistantText
            );
            assert_eq!(failure.source_method, "error");
            assert_eq!(failure.code, None);
            assert_eq!(failure.message, "model unavailable");
        }
        other => panic!("unexpected error: {other:?}"),
    }

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn run_prompt_surfaces_turn_failed_with_context() {
    let runtime = spawn_run_prompt_turn_failed_runtime().await;
    let err = runtime
        .run_prompt(PromptRunParams {
            cwd: "/tmp".to_owned(),
            prompt: "say ok".to_owned(),
            model: Some("gpt-5-codex".to_owned()),
            effort: Some(ReasoningEffort::High),
            approval_policy: ApprovalPolicy::Never,
            sandbox_policy: SandboxPolicy::Preset(SandboxPreset::ReadOnly),
            privileged_escalation_approved: false,
            attachments: vec![],
            timeout: Duration::from_secs(2),
        })
        .await
        .expect_err("run prompt must fail");

    match err {
        PromptRunError::TurnFailedWithContext(failure) => {
            assert_eq!(failure.terminal_state, PromptTurnTerminalState::Failed);
            assert_eq!(failure.source_method, "turn/failed");
            assert_eq!(failure.code, Some(429));
            assert_eq!(failure.message, "rate limited");
        }
        other => panic!("unexpected error: {other:?}"),
    }

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn run_prompt_timeout_uses_absolute_deadline_under_streaming_deltas() {
    let runtime = spawn_run_prompt_streaming_timeout_runtime().await;
    let timeout_value = Duration::from_millis(120);

    let started = Instant::now();
    let err = runtime
        .run_prompt(PromptRunParams::new("/tmp", "timeout probe").with_timeout(timeout_value))
        .await
        .expect_err("run prompt must timeout");

    assert!(matches!(err, PromptRunError::Timeout(d) if d == timeout_value));
    assert!(
        started.elapsed() < Duration::from_millis(350),
        "run_prompt exceeded expected absolute timeout window: {:?}",
        started.elapsed()
    );

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn run_prompt_recovers_when_live_stream_lags_past_terminal_event() {
    let runtime = spawn_run_prompt_lagged_completion_runtime().await;

    let result = runtime
        .run_prompt(PromptRunParams::new("/tmp", "lagged completion probe"))
        .await
        .expect("run prompt should recover from lagged stream");

    assert_eq!(result.thread_id, "thr_lagged");
    assert_eq!(result.turn_id, "turn_lagged");
    assert_eq!(result.assistant_text, "ok-from-thread-read");

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn run_prompt_timeout_emits_turn_interrupt_request() {
    let runtime = spawn_run_prompt_interrupt_probe_runtime().await;
    let mut live_rx = runtime.subscribe_live();
    let timeout_value = Duration::from_millis(120);

    let err = runtime
        .run_prompt(PromptRunParams::new("/tmp", "interrupt probe").with_timeout(timeout_value))
        .await
        .expect_err("run prompt must timeout");
    assert!(matches!(err, PromptRunError::Timeout(d) if d == timeout_value));

    let mut saw_interrupt = false;
    for _ in 0..16 {
        let envelope = tokio::time::timeout(Duration::from_secs(2), live_rx.recv())
            .await
            .expect("live timeout")
            .expect("live closed");
        if envelope.method.as_deref() == Some("probe/interruptSeen")
            && envelope.thread_id.as_deref() == Some("thr_interrupt_probe")
            && envelope.turn_id.as_deref() == Some("turn_interrupt_probe")
        {
            saw_interrupt = true;
            break;
        }
    }
    assert!(
        saw_interrupt,
        "timeout path must send turn/interrupt request"
    );

    runtime.shutdown().await.expect("shutdown");
}
