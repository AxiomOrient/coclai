use super::*;

fn seeded_store(temp: &TempDir, artifact_id: &str, text: &str) -> Arc<dyn ArtifactStore> {
    let store: Arc<dyn ArtifactStore> = Arc::new(FsArtifactStore::new(&temp.root));
    seed_artifact(store.as_ref(), artifact_id, text);
    store
}

async fn seeded_manager_with_mock_runtime(
    temp: &TempDir,
    artifact_id: &str,
    text: &str,
) -> (Arc<dyn ArtifactStore>, Runtime, ArtifactSessionManager) {
    let store = seeded_store(temp, artifact_id, text);
    let runtime = spawn_mock_runtime().await;
    let manager = ArtifactSessionManager::new(runtime.clone(), Arc::clone(&store));
    (store, runtime, manager)
}

#[tokio::test(flavor = "current_thread")]
async fn run_task_doc_generate_end_to_end() {
    let temp = TempDir::new("runtime_artifact_generate");
    let (store, runtime, manager) =
        seeded_manager_with_mock_runtime(&temp, "doc:generate", "").await;

    let spec = make_task_spec(
        "doc:generate",
        ArtifactTaskKind::DocGenerate,
        "GENERATE_DOC",
    );
    let result = manager.run_task(spec).await.expect("run task");

    match result {
        ArtifactTaskResult::DocGenerate {
            revision,
            text,
            title,
            ..
        } => {
            assert_eq!(title, "Generated Title");
            assert_eq!(text, "# Generated\ncontent\n");
            assert_eq!(revision, compute_revision(&text));
        }
        other => panic!("unexpected result: {other:?}"),
    }

    let persisted = store.load_text("doc:generate").expect("load persisted");
    assert_eq!(persisted, "# Generated\ncontent\n");

    let meta = store.get_meta("doc:generate").expect("load meta");
    assert_eq!(meta.title, "Generated Title");
    assert_eq!(meta.format, "markdown");
    assert_eq!(meta.revision, compute_revision(&persisted));
    assert_eq!(meta.runtime_thread_id.as_deref(), Some("thr_art"));

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn run_task_uses_artifact_adapter_boundary_without_runtime_dependency() {
    let temp = TempDir::new("runtime_artifact_fake_adapter");
    let store: Arc<dyn ArtifactStore> = Arc::new(FsArtifactStore::new(&temp.root));
    seed_artifact(store.as_ref(), "doc:adapter", "");

    let state = Arc::new(Mutex::new(FakeArtifactAdapterState {
        start_thread_id: "thr_fake_adapter".to_owned(),
        turn_output: json!({
            "format": "markdown",
            "title": "Adapter Title",
            "text": "# Adapter\nok\n"
        }),
        turn_id: Some("turn_fake_adapter".to_owned()),
        ..FakeArtifactAdapterState::default()
    }));
    let adapter: Arc<dyn ArtifactPluginAdapter> = Arc::new(FakeArtifactAdapter {
        state: Arc::clone(&state),
    });
    let manager = ArtifactSessionManager::new_with_adapter(adapter, Arc::clone(&store));

    let spec = make_task_spec("doc:adapter", ArtifactTaskKind::DocGenerate, "GENERATE_DOC");
    let result = manager
        .run_task(spec)
        .await
        .expect("run task with fake adapter");

    match result {
        ArtifactTaskResult::DocGenerate {
            thread_id,
            turn_id,
            title,
            text,
            ..
        } => {
            assert_eq!(thread_id, "thr_fake_adapter");
            assert_eq!(turn_id.as_deref(), Some("turn_fake_adapter"));
            assert_eq!(title, "Adapter Title");
            assert_eq!(text, "# Adapter\nok\n");
        }
        other => panic!("unexpected result: {other:?}"),
    }

    let state = state.lock().expect("fake adapter state");
    assert_eq!(state.start_calls, 1);
    assert!(state.resume_calls.is_empty());
    assert_eq!(state.run_turn_calls.len(), 1);
    let (thread_id, prompt, seen_spec) = &state.run_turn_calls[0];
    assert_eq!(thread_id, "thr_fake_adapter");
    assert!(prompt.contains("GOAL:\nGENERATE_DOC"));
    assert_eq!(seen_spec.artifact_id, "doc:adapter");
}

#[tokio::test(flavor = "current_thread")]
async fn run_task_doc_edit_end_to_end() {
    let temp = TempDir::new("runtime_artifact_edit");
    let (store, runtime, manager) =
        seeded_manager_with_mock_runtime(&temp, "doc:edit", "a\nb\nc\n").await;

    let spec = make_task_spec("doc:edit", ArtifactTaskKind::DocEdit, "EDIT_DOC");
    let result = manager.run_task(spec).await.expect("run task");

    match result {
        ArtifactTaskResult::DocEdit {
            text,
            notes,
            revision,
            ..
        } => {
            assert_eq!(text, "a\npatched\nc\n");
            assert_eq!(notes.as_deref(), Some("ok"));
            assert_eq!(revision, compute_revision("a\npatched\nc\n"));
        }
        other => panic!("unexpected result: {other:?}"),
    }

    let persisted = store.load_text("doc:edit").expect("load persisted");
    assert_eq!(persisted, "a\npatched\nc\n");

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn open_rejects_incompatible_adapter_contract() {
    let temp = TempDir::new("runtime_artifact_contract_mismatch");
    let store = seeded_store(&temp, "doc:contract", "seed\n");

    let adapter: Arc<dyn ArtifactPluginAdapter> = Arc::new(IncompatibleArtifactAdapter);
    let manager = ArtifactSessionManager::new_with_adapter(adapter, Arc::clone(&store));

    let err = manager
        .open("doc:contract")
        .await
        .expect_err("must reject mismatch");
    assert_eq!(
        err,
        DomainError::IncompatibleContract {
            expected_major: 1,
            expected_minor: 0,
            actual_major: 2,
            actual_minor: 0,
        }
    );
}

#[tokio::test(flavor = "current_thread")]
async fn open_accepts_compatible_minor_contract_version() {
    let temp = TempDir::new("runtime_artifact_contract_minor");
    let store = seeded_store(&temp, "doc:contract-minor", "seed\n");

    let adapter: Arc<dyn ArtifactPluginAdapter> = Arc::new(CompatibleMinorArtifactAdapter);
    let manager = ArtifactSessionManager::new_with_adapter(adapter, Arc::clone(&store));
    let session = manager
        .open("doc:contract-minor")
        .await
        .expect("minor version must remain compatible");

    assert_eq!(session.thread_id, "thr_contract_minor");
    assert_eq!(session.artifact_id, "doc:contract-minor");
}

#[tokio::test(flavor = "current_thread")]
async fn open_fails_when_resume_response_missing_thread_id() {
    let temp = TempDir::new("runtime_artifact_resume_missing_id");
    let store = seeded_store(&temp, "doc:resume-missing", "seed\n");

    let mut meta = store
        .get_meta("doc:resume-missing")
        .expect("seed meta must exist");
    meta.runtime_thread_id = Some("thr_existing".to_owned());
    store
        .set_meta("doc:resume-missing", meta)
        .expect("set runtime thread id");

    let runtime = spawn_resume_missing_id_runtime().await;
    let manager = ArtifactSessionManager::new(runtime.clone(), Arc::clone(&store));
    let err = manager
        .open("doc:resume-missing")
        .await
        .expect_err("open must fail on invalid resume response");

    match err {
        DomainError::Parse(message) => {
            assert!(message.contains("thread/resume missing thread id in result"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
    let meta = store
        .get_meta("doc:resume-missing")
        .expect("meta must remain readable");
    assert_eq!(meta.runtime_thread_id.as_deref(), Some("thr_existing"));

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn open_fails_when_resume_response_thread_id_mismatches_request() {
    let temp = TempDir::new("runtime_artifact_resume_mismatched_id");
    let store = seeded_store(&temp, "doc:resume-mismatch", "seed\n");

    let mut meta = store
        .get_meta("doc:resume-mismatch")
        .expect("seed meta must exist");
    meta.runtime_thread_id = Some("thr_existing".to_owned());
    store
        .set_meta("doc:resume-mismatch", meta)
        .expect("set runtime thread id");

    let runtime = spawn_resume_mismatched_id_runtime().await;
    let manager = ArtifactSessionManager::new(runtime.clone(), Arc::clone(&store));
    let err = manager
        .open("doc:resume-mismatch")
        .await
        .expect_err("open must fail on mismatched resume thread id");

    match err {
        DomainError::Parse(message) => {
            assert!(message.contains("thread/resume returned mismatched thread id"));
            assert!(message.contains("requested=thr_existing"));
            assert!(message.contains("actual=thr_unexpected"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
    let meta = store
        .get_meta("doc:resume-mismatch")
        .expect("meta must remain readable");
    assert_eq!(meta.runtime_thread_id.as_deref(), Some("thr_existing"));

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn conflict_is_returned_without_auto_retry() {
    let temp = TempDir::new("runtime_artifact_conflict");
    let (store, runtime, manager) =
        seeded_manager_with_mock_runtime(&temp, "doc:conflict", "a\nb\nc\n").await;

    let spec = make_task_spec("doc:conflict", ArtifactTaskKind::DocEdit, "EDIT_CONFLICT");
    let err = manager.run_task(spec).await.expect_err("must conflict");

    match err {
        DomainError::Conflict { expected, actual } => {
            assert_eq!(expected, "sha256:deadbeef");
            assert_eq!(actual, compute_revision("a\nb\nc\n"));
        }
        other => panic!("unexpected error: {other:?}"),
    }

    let persisted = store.load_text("doc:conflict").expect("load persisted");
    assert_eq!(persisted, "a\nb\nc\n");
    let meta = store.get_meta("doc:conflict").expect("load meta");
    assert_eq!(meta.revision, compute_revision("a\nb\nc\n"));

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn turn_start_params_use_fixed_safe_policy() {
    let temp = TempDir::new("runtime_artifact_policy");
    let (_store, runtime, manager) =
        seeded_manager_with_mock_runtime(&temp, "doc:policy", "seed\n").await;

    let spec = make_task_spec("doc:policy", ArtifactTaskKind::Passthrough, "POLICY_CHECK");
    let result = manager.run_task(spec).await.expect("run task");

    match result {
        ArtifactTaskResult::Passthrough { output, .. } => {
            assert_eq!(output["approvalPolicy"], "never");
            assert_eq!(output["sandboxPolicy"]["type"], "readOnly");
        }
        other => panic!("unexpected result: {other:?}"),
    }

    runtime.shutdown().await.expect("shutdown");
}

#[tokio::test(flavor = "current_thread")]
async fn run_task_sends_interrupt_when_output_collection_fails() {
    let temp = TempDir::new("runtime_artifact_interrupt_probe");
    let store = seeded_store(&temp, "doc:interrupt", "seed\n");

    let interrupt_mark = temp.root.join("interrupt_seen.txt");
    let interrupt_mark_str = interrupt_mark.to_string_lossy().to_string();
    let runtime = spawn_interrupt_probe_runtime(&interrupt_mark_str).await;
    let manager = ArtifactSessionManager::new(runtime.clone(), Arc::clone(&store));

    let spec = make_task_spec(
        "doc:interrupt",
        ArtifactTaskKind::Passthrough,
        "INTERRUPT_CHECK",
    );
    let err = manager.run_task(spec).await.expect_err("must fail");
    assert!(matches!(err, DomainError::Parse(_)));
    assert!(
        interrupt_mark.exists(),
        "run_task failure must emit turn/interrupt best effort"
    );
    let persisted = store.load_text("doc:interrupt").expect("persisted text");
    assert_eq!(persisted, "seed\n");
    let meta = store.get_meta("doc:interrupt").expect("persisted meta");
    assert_eq!(meta.revision, compute_revision("seed\n"));

    runtime.shutdown().await.expect("shutdown");
}
