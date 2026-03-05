use std::path::PathBuf;

use crate::runtime::api::ReasoningEffort;
use crate::runtime::errors::{RpcError, RuntimeError};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;

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
    fn save_text_and_meta(
        &self,
        artifact_id: &str,
        new_text: &str,
        save_meta: SaveMeta,
        meta: ArtifactMeta,
    ) -> Result<(), StoreErr> {
        self.save_text(artifact_id, new_text, save_meta)?;
        self.set_meta(artifact_id, meta)
    }
    fn get_meta(&self, artifact_id: &str) -> Result<ArtifactMeta, StoreErr>;
    fn set_meta(&self, artifact_id: &str, meta: ArtifactMeta) -> Result<(), StoreErr>;
}

#[derive(Clone, Debug)]
pub struct FsArtifactStore {
    pub(super) root: PathBuf,
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

// --- from patch.rs ---

pub(crate) fn map_patch_conflict(conflict: PatchConflict) -> DomainError {
    match conflict {
        PatchConflict::RevisionMismatch { expected, actual } => {
            DomainError::Conflict { expected, actual }
        }
        other => DomainError::Validation(other.to_string()),
    }
}

pub fn compute_revision(text: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(text.as_bytes());
    format!("sha256:{}", hex::encode(hasher.finalize()))
}

/// Validate structural patch invariants before any mutation.
/// `end_line` is exclusive.
/// Allocation: clones only the validated edit list. Complexity: O(e), e = edit count.
pub fn validate_doc_patch(text: &str, patch: &DocPatch) -> Result<ValidatedPatch, PatchConflict> {
    let current_revision = compute_revision(text);
    if current_revision != patch.expected_revision {
        return Err(PatchConflict::RevisionMismatch {
            expected: patch.expected_revision.clone(),
            actual: current_revision,
        });
    }

    let line_count = line_count(text);
    let mut prev_start = 0usize;
    let mut prev_end = 0usize;

    for (index, edit) in patch.edits.iter().enumerate() {
        if edit.start_line == 0
            || edit.start_line > edit.end_line
            || edit.end_line > line_count.saturating_add(1)
        {
            return Err(PatchConflict::InvalidRange {
                index,
                start_line: edit.start_line,
                end_line: edit.end_line,
                line_count,
            });
        }

        if index > 0 {
            if edit.start_line < prev_start {
                return Err(PatchConflict::NotSorted {
                    index,
                    prev_start,
                    start: edit.start_line,
                });
            }
            if edit.start_line < prev_end {
                return Err(PatchConflict::Overlap {
                    index,
                    prev_end,
                    start: edit.start_line,
                });
            }
        }

        prev_start = edit.start_line;
        prev_end = edit.end_line;
    }

    Ok(ValidatedPatch {
        edits: patch.edits.clone(),
    })
}

/// Apply validated edits in reverse order to avoid index drift.
/// Allocation: line buffer + replacement line buffers. Complexity: O(L + R + e).
pub fn apply_doc_patch(text: &str, validated_patch: &ValidatedPatch) -> String {
    let mut lines = split_lines(text);

    for edit in validated_patch.edits.iter().rev() {
        let start_idx = edit.start_line.saturating_sub(1);
        let end_idx = edit.end_line.saturating_sub(1);
        let replacement = split_lines(&edit.replacement);
        lines.splice(start_idx..end_idx, replacement);
    }

    lines.concat()
}

fn line_count(text: &str) -> usize {
    split_lines(text).len()
}

fn split_lines(text: &str) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    text.split_inclusive('\n').map(ToOwned::to_owned).collect()
}
