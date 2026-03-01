use std::path::PathBuf;

use coclai_runtime::api::ReasoningEffort;
use coclai_runtime::errors::{RpcError, RuntimeError};
use serde::{Deserialize, Serialize};
use serde_json::Value;
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
