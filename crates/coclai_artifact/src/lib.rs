use std::path::PathBuf;
use std::sync::Arc;

use coclai_runtime::api::ReasoningEffort;
use coclai_runtime::errors::{RpcError, RuntimeError};
use coclai_runtime::runtime::Runtime;
use coclai_runtime::PluginContractVersion;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

mod adapter;
mod patch;
mod store;
mod task;

pub use adapter::{
    ArtifactAdapterFuture, ArtifactPluginAdapter, ArtifactTurnOutput, RuntimeArtifactAdapter,
};
pub use patch::{apply_doc_patch, compute_revision, validate_doc_patch};
pub use task::{build_turn_prompt, build_turn_start_params};

use patch::map_patch_conflict;
#[cfg(test)]
pub(crate) use store::artifact_key;
#[cfg(test)]
pub(crate) use task::collect_turn_output_from_live_with_limits;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactMeta {
    pub title: String,
    pub format: String,
    pub revision: String,
    pub runtime_thread_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ArtifactTaskKind {
    DocGenerate,
    DocEdit,
    Passthrough,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactTaskSpec {
    pub artifact_id: String,
    pub kind: ArtifactTaskKind,
    pub user_goal: String,
    pub current_text: Option<String>,
    pub constraints: Vec<String>,
    pub examples: Vec<String>,
    pub model: Option<String>,
    pub effort: Option<ReasoningEffort>,
    pub summary: Option<String>,
    pub output_schema: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DocPatch {
    pub format: String,
    pub expected_revision: String,
    pub edits: Vec<DocEdit>,
    pub notes: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DocEdit {
    pub start_line: usize,
    pub end_line: usize,
    pub replacement: String,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum PatchConflict {
    #[error("expected revision mismatch: expected={expected} actual={actual}")]
    RevisionMismatch { expected: String, actual: String },
    #[error(
        "invalid range at edit#{index}: start={start_line} end={end_line} line_count={line_count}"
    )]
    InvalidRange {
        index: usize,
        start_line: usize,
        end_line: usize,
        line_count: usize,
    },
    #[error("edits are not sorted at edit#{index}: prev_start={prev_start} start={start}")]
    NotSorted {
        index: usize,
        prev_start: usize,
        start: usize,
    },
    #[error("edits overlap at edit#{index}: prev_end={prev_end} start={start}")]
    Overlap {
        index: usize,
        prev_end: usize,
        start: usize,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidatedPatch {
    pub edits: Vec<DocEdit>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SaveMeta {
    pub task_kind: ArtifactTaskKind,
    pub thread_id: String,
    pub turn_id: Option<String>,
    pub previous_revision: Option<String>,
    pub next_revision: String,
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum StoreErr {
    #[error("artifact not found: {0}")]
    NotFound(String),
    #[error("store conflict: expected={expected} actual={actual}")]
    Conflict { expected: String, actual: String },
    #[error("io error: {0}")]
    Io(String),
    #[error("serialize error: {0}")]
    Serialize(String),
}

pub trait ArtifactStore: Send + Sync {
    fn load_text(&self, artifact_id: &str) -> Result<String, StoreErr>;
    fn save_text(&self, artifact_id: &str, new_text: &str, meta: SaveMeta) -> Result<(), StoreErr>;
    fn get_meta(&self, artifact_id: &str) -> Result<ArtifactMeta, StoreErr>;
    fn set_meta(&self, artifact_id: &str, meta: ArtifactMeta) -> Result<(), StoreErr>;
}

#[derive(Clone, Debug)]
pub struct FsArtifactStore {
    root: PathBuf,
}

#[derive(Clone, Debug, Error, PartialEq)]
pub enum DomainError {
    #[error("conflict: expected={expected} actual={actual}")]
    Conflict { expected: String, actual: String },
    #[error(
        "incompatible plugin contract: expected=v{expected_major}.{expected_minor} actual=v{actual_major}.{actual_minor}"
    )]
    IncompatibleContract {
        expected_major: u16,
        expected_minor: u16,
        actual_major: u16,
        actual_minor: u16,
    },
    #[error("validation error: {0}")]
    Validation(String),
    #[error("parse error: {0}")]
    Parse(String),
    #[error("store error: {0}")]
    Store(StoreErr),
    #[error("rpc error: {0}")]
    Rpc(#[from] RpcError),
    #[error("runtime error: {0}")]
    Runtime(#[from] RuntimeError),
}

impl From<StoreErr> for DomainError {
    fn from(value: StoreErr) -> Self {
        match value {
            StoreErr::Conflict { expected, actual } => Self::Conflict { expected, actual },
            other => Self::Store(other),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtifactSession {
    pub artifact_id: String,
    pub thread_id: String,
    pub format: String,
    pub revision: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum ArtifactTaskResult {
    DocGenerate {
        artifact_id: String,
        thread_id: String,
        turn_id: Option<String>,
        title: String,
        format: String,
        revision: String,
        text: String,
    },
    DocEdit {
        artifact_id: String,
        thread_id: String,
        turn_id: Option<String>,
        format: String,
        revision: String,
        text: String,
        notes: Option<String>,
    },
    Passthrough {
        artifact_id: String,
        thread_id: String,
        turn_id: Option<String>,
        output: Value,
    },
}

#[derive(Clone)]
pub struct ArtifactSessionManager {
    adapter: Arc<dyn ArtifactPluginAdapter>,
    store: Arc<dyn ArtifactStore>,
    contract_mismatch: Option<ContractMismatch>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ContractMismatch {
    expected: PluginContractVersion,
    actual: PluginContractVersion,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DocGenerateOutput {
    format: String,
    title: String,
    text: String,
}

impl ArtifactSessionManager {
    pub fn new(runtime: Runtime, store: Arc<dyn ArtifactStore>) -> Self {
        let adapter: Arc<dyn ArtifactPluginAdapter> =
            Arc::new(RuntimeArtifactAdapter::new(runtime));
        Self::new_with_adapter(adapter, store)
    }

    pub fn new_with_adapter(
        adapter: Arc<dyn ArtifactPluginAdapter>,
        store: Arc<dyn ArtifactStore>,
    ) -> Self {
        let contract_mismatch = detect_contract_mismatch(adapter.as_ref());
        Self {
            adapter,
            store,
            contract_mismatch,
        }
    }

    async fn store_io<T: Send + 'static>(
        &self,
        op: impl FnOnce(&dyn ArtifactStore) -> Result<T, StoreErr> + Send + 'static,
    ) -> Result<T, DomainError> {
        let store = Arc::clone(&self.store);
        let joined = tokio::task::spawn_blocking(move || op(store.as_ref()))
            .await
            .map_err(|err| {
                DomainError::Store(StoreErr::Io(format!("store worker join failed: {err}")))
            })?;
        joined.map_err(DomainError::from)
    }

    pub async fn open(&self, artifact_id: &str) -> Result<ArtifactSession, DomainError> {
        self.ensure_contract_compatible()?;

        let artifact_id_owned = artifact_id.to_owned();
        let mut meta = self
            .store_io({
                let artifact_id = artifact_id_owned.clone();
                move |store| task::load_or_default_meta(store, &artifact_id)
            })
            .await?;

        let thread_id = match meta.runtime_thread_id.as_deref() {
            Some(existing) => self.adapter.resume_thread(existing).await?,
            None => self.adapter.start_thread().await?,
        };
        meta.runtime_thread_id = Some(thread_id.clone());
        self.store_io({
            let artifact_id = artifact_id_owned.clone();
            let meta_to_store = meta.clone();
            move |store| store.set_meta(&artifact_id, meta_to_store)
        })
        .await?;

        Ok(ArtifactSession {
            artifact_id: artifact_id_owned,
            thread_id,
            format: meta.format,
            revision: meta.revision,
        })
    }

    /// Domain task runner with explicit side-effect boundary:
    /// runtime RPC call + store read/write.
    /// Allocation: prompt string + output JSON parse structures.
    /// Complexity: O(L + e) for DocEdit (L=text size, e=edit count).
    pub async fn run_task(
        &self,
        spec: ArtifactTaskSpec,
    ) -> Result<ArtifactTaskResult, DomainError> {
        self.ensure_contract_compatible()?;
        let session = self.open(&spec.artifact_id).await?;

        let persisted_text = match self
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
        let prompt =
            task::build_turn_prompt(&spec, &session.format, &persisted_revision, context_text);
        let turn_output = self
            .adapter
            .run_turn(&session.thread_id, &prompt, &spec)
            .await?;
        let turn_id = turn_output.turn_id;
        let turn_output = turn_output.output;

        match spec.kind {
            ArtifactTaskKind::DocGenerate => {
                let output_json =
                    task::extract_output_json(&turn_output, &["format", "title", "text"])?;
                let output: DocGenerateOutput =
                    serde_json::from_value(output_json).map_err(|err| {
                        DomainError::Parse(format!("docGenerate payload parse failed: {err}"))
                    })?;

                let new_revision = compute_revision(&output.text);
                let mut meta = self
                    .store_io({
                        let artifact_id = spec.artifact_id.clone();
                        move |store| store.get_meta(&artifact_id)
                    })
                    .await?;
                self.store_io({
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
                self.store_io({
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
            ArtifactTaskKind::DocEdit => {
                let output_json = task::extract_output_json(
                    &turn_output,
                    &["format", "expectedRevision", "edits"],
                )?;
                let patch: DocPatch = serde_json::from_value(output_json).map_err(|err| {
                    DomainError::Parse(format!("docEdit patch parse failed: {err}"))
                })?;

                let validated =
                    validate_doc_patch(&persisted_text, &patch).map_err(map_patch_conflict)?;
                let new_text = apply_doc_patch(&persisted_text, &validated);
                let new_revision = compute_revision(&new_text);

                let mut meta = self
                    .store_io({
                        let artifact_id = spec.artifact_id.clone();
                        move |store| store.get_meta(&artifact_id)
                    })
                    .await?;
                self.store_io({
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
                self.store_io({
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
            ArtifactTaskKind::Passthrough => Ok(ArtifactTaskResult::Passthrough {
                artifact_id: spec.artifact_id,
                thread_id: session.thread_id,
                turn_id,
                output: turn_output,
            }),
        }
    }

    fn ensure_contract_compatible(&self) -> Result<(), DomainError> {
        if let Some(mismatch) = self.contract_mismatch {
            return Err(DomainError::IncompatibleContract {
                expected_major: mismatch.expected.major,
                expected_minor: mismatch.expected.minor,
                actual_major: mismatch.actual.major,
                actual_minor: mismatch.actual.minor,
            });
        }
        Ok(())
    }
}

fn detect_contract_mismatch(adapter: &dyn ArtifactPluginAdapter) -> Option<ContractMismatch> {
    let expected = PluginContractVersion::CURRENT;
    let actual = adapter.plugin_contract_version();
    if expected.is_compatible_with(actual) {
        None
    } else {
        Some(ContractMismatch { expected, actual })
    }
}
#[cfg(test)]
mod tests;
