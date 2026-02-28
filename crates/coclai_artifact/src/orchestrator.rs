use serde::Deserialize;
use serde_json::Value;

use super::patch::map_patch_conflict;
use super::{
    apply_doc_patch, compute_revision, task, validate_doc_patch, ArtifactSession,
    ArtifactSessionManager, ArtifactTaskKind, ArtifactTaskResult, ArtifactTaskSpec, DocPatch,
    DomainError, SaveMeta, StoreErr,
};

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DocGenerateOutput {
    format: String,
    title: String,
    text: String,
}

pub(super) async fn run_task(
    manager: &ArtifactSessionManager,
    spec: ArtifactTaskSpec,
) -> Result<ArtifactTaskResult, DomainError> {
    manager.ensure_contract_compatible()?;
    let session = manager.open(&spec.artifact_id).await?;

    let persisted_text = match manager
        .store_io({
            let artifact_id = spec.artifact_id.clone();
            move |store| store.load_text(&artifact_id)
        })
        .await
    {
        Ok(text) => text,
        Err(DomainError::Store(StoreErr::NotFound(_))) => String::new(),
        Err(err) => return Err(err),
    };
    let persisted_revision = compute_revision(&persisted_text);

    let context_text = spec.current_text.as_deref().unwrap_or(&persisted_text);
    let prompt = task::build_turn_prompt(&spec, &session.format, &persisted_revision, context_text);
    let turn_output = manager
        .adapter
        .run_turn(&session.thread_id, &prompt, &spec)
        .await?;
    let turn_id = turn_output.turn_id;
    let turn_output = turn_output.output;

    match spec.kind {
        ArtifactTaskKind::DocGenerate => {
            run_doc_generate(
                manager,
                spec,
                session,
                persisted_revision,
                turn_id,
                turn_output,
            )
            .await
        }
        ArtifactTaskKind::DocEdit => {
            run_doc_edit(
                manager,
                spec,
                session,
                persisted_text,
                persisted_revision,
                turn_id,
                turn_output,
            )
            .await
        }
        ArtifactTaskKind::Passthrough => Ok(ArtifactTaskResult::Passthrough {
            artifact_id: spec.artifact_id,
            thread_id: session.thread_id,
            turn_id,
            output: turn_output,
        }),
    }
}

async fn run_doc_generate(
    manager: &ArtifactSessionManager,
    spec: ArtifactTaskSpec,
    session: ArtifactSession,
    persisted_revision: String,
    turn_id: Option<String>,
    turn_output: Value,
) -> Result<ArtifactTaskResult, DomainError> {
    let output_json = task::extract_output_json(&turn_output, &["format", "title", "text"])?;
    let output: DocGenerateOutput = serde_json::from_value(output_json)
        .map_err(|err| DomainError::Parse(format!("docGenerate payload parse failed: {err}")))?;

    let new_revision = compute_revision(&output.text);
    let mut meta = manager
        .store_io({
            let artifact_id = spec.artifact_id.clone();
            move |store| store.get_meta(&artifact_id)
        })
        .await?;
    manager
        .store_io({
            let artifact_id = spec.artifact_id.clone();
            let output_text = output.text.clone();
            let save_meta = SaveMeta {
                task_kind: ArtifactTaskKind::DocGenerate,
                thread_id: session.thread_id.clone(),
                turn_id: turn_id.clone(),
                previous_revision: Some(persisted_revision.clone()),
                next_revision: new_revision.clone(),
            };
            move |store| store.save_text(&artifact_id, &output_text, save_meta)
        })
        .await?;
    meta.title = output.title.clone();
    meta.format = output.format.clone();
    meta.revision = new_revision.clone();
    meta.runtime_thread_id = Some(session.thread_id.clone());
    manager
        .store_io({
            let artifact_id = spec.artifact_id.clone();
            move |store| store.set_meta(&artifact_id, meta)
        })
        .await?;

    Ok(ArtifactTaskResult::DocGenerate {
        artifact_id: spec.artifact_id,
        thread_id: session.thread_id,
        turn_id,
        title: output.title,
        format: output.format,
        revision: new_revision,
        text: output.text,
    })
}

async fn run_doc_edit(
    manager: &ArtifactSessionManager,
    spec: ArtifactTaskSpec,
    session: ArtifactSession,
    persisted_text: String,
    persisted_revision: String,
    turn_id: Option<String>,
    turn_output: Value,
) -> Result<ArtifactTaskResult, DomainError> {
    let output_json =
        task::extract_output_json(&turn_output, &["format", "expectedRevision", "edits"])?;
    let patch: DocPatch = serde_json::from_value(output_json)
        .map_err(|err| DomainError::Parse(format!("docEdit patch parse failed: {err}")))?;

    let validated = validate_doc_patch(&persisted_text, &patch).map_err(map_patch_conflict)?;
    let new_text = apply_doc_patch(&persisted_text, &validated);
    let new_revision = compute_revision(&new_text);

    let mut meta = manager
        .store_io({
            let artifact_id = spec.artifact_id.clone();
            move |store| store.get_meta(&artifact_id)
        })
        .await?;
    manager
        .store_io({
            let artifact_id = spec.artifact_id.clone();
            let new_text_to_save = new_text.clone();
            let save_meta = SaveMeta {
                task_kind: ArtifactTaskKind::DocEdit,
                thread_id: session.thread_id.clone(),
                turn_id: turn_id.clone(),
                previous_revision: Some(persisted_revision.clone()),
                next_revision: new_revision.clone(),
            };
            move |store| store.save_text(&artifact_id, &new_text_to_save, save_meta)
        })
        .await?;
    meta.format = patch.format.clone();
    meta.revision = new_revision.clone();
    meta.runtime_thread_id = Some(session.thread_id.clone());
    manager
        .store_io({
            let artifact_id = spec.artifact_id.clone();
            move |store| store.set_meta(&artifact_id, meta)
        })
        .await?;

    Ok(ArtifactTaskResult::DocEdit {
        artifact_id: spec.artifact_id,
        thread_id: session.thread_id,
        turn_id,
        format: patch.format,
        revision: new_revision,
        text: new_text,
        notes: patch.notes,
    })
}
