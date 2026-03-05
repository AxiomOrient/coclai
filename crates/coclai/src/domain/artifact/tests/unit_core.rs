use super::*;

#[test]
fn build_prompt_has_required_blocks() {
    let spec = ArtifactTaskSpec {
        artifact_id: "doc:prompt".to_owned(),
        kind: ArtifactTaskKind::Passthrough,
        user_goal: "goal".to_owned(),
        current_text: None,
        constraints: vec!["c1".to_owned()],
        examples: vec!["ex1".to_owned()],
        model: None,
        effort: None,
        summary: None,
        output_schema: json!({"type":"object"}),
    };
    let prompt = build_turn_prompt(&spec, "markdown", "sha256:rev", "hello\n");
    assert!(prompt.contains("ROLE:\n"));
    assert!(prompt.contains("GOAL:\n"));
    assert!(prompt.contains("CONSTRAINTS:\n"));
    assert!(prompt.contains("CONTEXT:\n"));
    assert!(prompt.contains("REVISION: sha256:rev"));
    assert!(prompt.contains("CURRENT_TEXT_BEGIN\nhello\nCURRENT_TEXT_END"));
}

#[test]
fn parse_ids_support_nested_structures() {
    assert_eq!(
        parse_thread_id(&json!({"thread":{"id":"thr_nested"}})).as_deref(),
        Some("thr_nested")
    );
    assert_eq!(
        parse_turn_id(&json!({"turn":{"id":"turn_nested"}})).as_deref(),
        Some("turn_nested")
    );
}

#[test]
fn validate_and_apply_replace() {
    let before = "a\nb\nc\n";
    let patch = DocPatch {
        format: "markdown".to_owned(),
        expected_revision: compute_revision(before),
        edits: vec![DocEdit {
            start_line: 2,
            end_line: 3,
            replacement: "B\n".to_owned(),
        }],
        notes: None,
    };

    let validated = validate_doc_patch(before, &patch).expect("valid patch");
    let after = apply_doc_patch(before, &validated);
    assert_eq!(after, "a\nB\nc\n");
}

#[test]
fn validate_insert_head_and_append() {
    let before = "line1\nline2\n";
    let patch = DocPatch {
        format: "markdown".to_owned(),
        expected_revision: compute_revision(before),
        edits: vec![
            DocEdit {
                start_line: 1,
                end_line: 1,
                replacement: "head\n".to_owned(),
            },
            DocEdit {
                start_line: 3,
                end_line: 3,
                replacement: "tail\n".to_owned(),
            },
        ],
        notes: None,
    };

    let validated = validate_doc_patch(before, &patch).expect("valid patch");
    let after = apply_doc_patch(before, &validated);
    assert_eq!(after, "head\nline1\nline2\ntail\n");
}

#[test]
fn detect_revision_conflict() {
    let before = "a\n";
    let patch = DocPatch {
        format: "markdown".to_owned(),
        expected_revision: "sha256:deadbeef".to_owned(),
        edits: vec![],
        notes: None,
    };

    let err = validate_doc_patch(before, &patch).expect_err("must fail");
    assert!(matches!(err, PatchConflict::RevisionMismatch { .. }));
}

#[test]
fn detect_overlap() {
    let before = "a\nb\nc\n";
    let patch = DocPatch {
        format: "markdown".to_owned(),
        expected_revision: compute_revision(before),
        edits: vec![
            DocEdit {
                start_line: 1,
                end_line: 3,
                replacement: "x\n".to_owned(),
            },
            DocEdit {
                start_line: 2,
                end_line: 3,
                replacement: "y\n".to_owned(),
            },
        ],
        notes: None,
    };

    let err = validate_doc_patch(before, &patch).expect_err("must fail");
    assert!(matches!(err, PatchConflict::Overlap { .. }));
}

#[test]
fn detect_invalid_range() {
    let before = "a\n";
    let patch = DocPatch {
        format: "markdown".to_owned(),
        expected_revision: compute_revision(before),
        edits: vec![DocEdit {
            start_line: 2,
            end_line: 4,
            replacement: "x\n".to_owned(),
        }],
        notes: None,
    };

    let err = validate_doc_patch(before, &patch).expect_err("must fail");
    assert!(matches!(err, PatchConflict::InvalidRange { .. }));
}

#[test]
fn artifact_key_is_stable() {
    let a = artifact_key("doc:123");
    let b = artifact_key("doc:123");
    let c = artifact_key("doc/123");
    assert_eq!(a, b);
    assert_ne!(a, c);
}

#[test]
fn fs_store_rejects_stale_revision_on_save() {
    let temp = TempDir::new("runtime_artifact_store_conflict");
    let store = FsArtifactStore::new(&temp.root);

    let base_text = "v1\n";
    let base_revision = compute_revision(base_text);
    store
        .save_text(
            "doc:store-conflict",
            base_text,
            SaveMeta {
                task_kind: ArtifactTaskKind::DocGenerate,
                thread_id: "seed".to_owned(),
                turn_id: None,
                previous_revision: None,
                next_revision: base_revision.clone(),
            },
        )
        .expect("seed save");

    let next_text = "v2\n";
    let next_revision = compute_revision(next_text);
    store
        .save_text(
            "doc:store-conflict",
            next_text,
            SaveMeta {
                task_kind: ArtifactTaskKind::DocEdit,
                thread_id: "seed".to_owned(),
                turn_id: Some("turn_1".to_owned()),
                previous_revision: Some(base_revision.clone()),
                next_revision,
            },
        )
        .expect("first update");

    let stale = store
        .save_text(
            "doc:store-conflict",
            "v3\n",
            SaveMeta {
                task_kind: ArtifactTaskKind::DocEdit,
                thread_id: "seed".to_owned(),
                turn_id: Some("turn_2".to_owned()),
                previous_revision: Some(base_revision),
                next_revision: compute_revision("v3\n"),
            },
        )
        .expect_err("stale save must fail");
    assert!(matches!(stale, StoreErr::Conflict { .. }));
}

#[test]
fn fs_store_recovers_orphaned_lock_and_saves() {
    let temp = TempDir::new("runtime_artifact_store_orphaned_lock");
    let store = FsArtifactStore::new(&temp.root);
    let artifact_id = "doc:orphaned-lock";

    let artifact_dir = temp.root.join(artifact_key(artifact_id));
    fs::create_dir_all(&artifact_dir).expect("create artifact dir");
    fs::write(artifact_dir.join(".artifact.lock"), "999999:1\n").expect("write orphaned lock");

    let next_text = "v1\n";
    let next_revision = compute_revision(next_text);
    store
        .save_text(
            artifact_id,
            next_text,
            SaveMeta {
                task_kind: ArtifactTaskKind::DocGenerate,
                thread_id: "seed".to_owned(),
                turn_id: None,
                previous_revision: None,
                next_revision: next_revision.clone(),
            },
        )
        .expect("save must recover orphaned lock");

    let persisted = store.load_text(artifact_id).expect("load persisted");
    assert_eq!(persisted, next_text);
    assert!(!artifact_dir.join(".artifact.lock").exists());
}

#[test]
fn fs_store_does_not_steal_live_lock_owner() {
    let temp = TempDir::new("runtime_artifact_store_live_lock_owner");
    let store = FsArtifactStore::new(&temp.root);
    let artifact_id = "doc:live-lock-owner";

    let artifact_dir = temp.root.join(artifact_key(artifact_id));
    fs::create_dir_all(&artifact_dir).expect("create artifact dir");
    let current_pid = std::process::id();
    fs::write(
        artifact_dir.join(".artifact.lock"),
        format!("{current_pid}:1\n"),
    )
    .expect("write live-owner lock");

    let err = store
        .save_text(
            artifact_id,
            "v1\n",
            SaveMeta {
                task_kind: ArtifactTaskKind::DocGenerate,
                thread_id: "seed".to_owned(),
                turn_id: None,
                previous_revision: None,
                next_revision: compute_revision("v1\n"),
            },
        )
        .expect_err("active owner lock must not be stolen");
    match err {
        StoreErr::Io(message) => assert!(message.contains("artifact lock timed out")),
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn fs_store_rejects_meta_revision_mismatch() {
    let temp = TempDir::new("runtime_artifact_store_meta_conflict");
    let store = FsArtifactStore::new(&temp.root);

    let text = "body\n";
    let revision = compute_revision(text);
    store
        .save_text(
            "doc:meta-conflict",
            text,
            SaveMeta {
                task_kind: ArtifactTaskKind::DocGenerate,
                thread_id: "seed".to_owned(),
                turn_id: None,
                previous_revision: None,
                next_revision: revision.clone(),
            },
        )
        .expect("seed save");

    let err = store
        .set_meta(
            "doc:meta-conflict",
            ArtifactMeta {
                title: "x".to_owned(),
                format: "markdown".to_owned(),
                revision: "sha256:deadbeef".to_owned(),
                runtime_thread_id: None,
            },
        )
        .expect_err("meta revision mismatch must fail");
    assert!(matches!(err, StoreErr::Conflict { .. }));
}

#[test]
fn fs_store_save_text_and_meta_rolls_back_text_when_meta_write_fails() {
    let temp = TempDir::new("runtime_artifact_store_atomic_rollback");
    let store = FsArtifactStore::new(&temp.root);
    let artifact_id = "doc:atomic-rollback";

    let seed_text = "v1\n";
    let seed_revision = compute_revision(seed_text);
    store
        .save_text(
            artifact_id,
            seed_text,
            SaveMeta {
                task_kind: ArtifactTaskKind::DocGenerate,
                thread_id: "seed".to_owned(),
                turn_id: None,
                previous_revision: None,
                next_revision: seed_revision.clone(),
            },
        )
        .expect("seed save");
    store
        .set_meta(
            artifact_id,
            ArtifactMeta {
                title: "Seed".to_owned(),
                format: "markdown".to_owned(),
                revision: seed_revision.clone(),
                runtime_thread_id: None,
            },
        )
        .expect("seed meta");

    let dir = temp.root.join(artifact_key(artifact_id));
    let meta_path = dir.join("meta.json");
    fs::remove_file(&meta_path).expect("remove meta file");
    fs::create_dir(&meta_path).expect("create blocking meta dir");

    let next_text = "v2\n";
    let next_revision = compute_revision(next_text);
    let err = store
        .save_text_and_meta(
            artifact_id,
            next_text,
            SaveMeta {
                task_kind: ArtifactTaskKind::DocEdit,
                thread_id: "seed".to_owned(),
                turn_id: Some("turn_1".to_owned()),
                previous_revision: Some(seed_revision),
                next_revision: next_revision.clone(),
            },
            ArtifactMeta {
                title: "Seed".to_owned(),
                format: "markdown".to_owned(),
                revision: next_revision,
                runtime_thread_id: Some("thr_seed".to_owned()),
            },
        )
        .expect_err("meta persist failure must fail atomic save");
    assert!(matches!(err, StoreErr::Io(_)));

    let persisted = store.load_text(artifact_id).expect("load persisted text");
    assert_eq!(persisted, seed_text);
}

#[tokio::test(flavor = "current_thread")]
async fn open_repairs_meta_revision_mismatch() {
    let temp = TempDir::new("runtime_artifact_open_repair");
    let store: Arc<dyn ArtifactStore> = Arc::new(FsArtifactStore::new(&temp.root));

    let artifact_id = "doc:meta-mismatch";
    let key = artifact_key(artifact_id);
    let dir = temp.root.join(key);
    fs::create_dir_all(&dir).expect("create dir");
    fs::write(dir.join("text.txt"), "seed\n").expect("write text");
    fs::write(
        dir.join("meta.json"),
        serde_json::to_vec(&ArtifactMeta {
            title: "Seed".to_owned(),
            format: "markdown".to_owned(),
            revision: "sha256:bad".to_owned(),
            runtime_thread_id: None,
        })
        .expect("serialize meta"),
    )
    .expect("write meta");

    let runtime = spawn_mock_runtime().await;
    let manager = ArtifactSessionManager::new(runtime.clone(), Arc::clone(&store));
    let opened = manager.open(artifact_id).await.expect("open must repair");
    assert_eq!(opened.revision, compute_revision("seed\n"));

    let meta = store.get_meta(artifact_id).expect("meta after open");
    assert_eq!(meta.revision, compute_revision("seed\n"));
    assert_eq!(meta.runtime_thread_id.as_deref(), Some("thr_art"));

    runtime.shutdown().await.expect("shutdown");
}

#[test]
fn build_turn_start_params_defaults_effort_to_medium() {
    let spec = ArtifactTaskSpec {
        artifact_id: "doc:default-effort".to_owned(),
        kind: ArtifactTaskKind::Passthrough,
        user_goal: "goal".to_owned(),
        current_text: None,
        constraints: vec![],
        examples: vec![],
        model: None,
        effort: None,
        summary: None,
        output_schema: json!({"type":"object"}),
    };
    let params = build_turn_start_params("thr_1", "prompt", &spec);
    assert_eq!(params["effort"], "medium");
}
