use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::str::FromStr;

use crate::runtime::core::Runtime;

use super::input::{InputItem, ThreadId, TurnId};
use super::policies::{ApprovalPolicy, ReasoningEffort, SandboxPolicy};
use super::thread_views::ThreadView;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TurnStartParams {
    pub input: Vec<InputItem>,
    pub cwd: Option<String>,
    pub approval_policy: Option<ApprovalPolicy>,
    pub sandbox_policy: Option<SandboxPolicy>,
    pub privileged_escalation_approved: bool,
    pub model: Option<String>,
    pub effort: Option<ReasoningEffort>,
    pub summary: Option<String>,
    pub output_schema: Option<Value>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TurnHandle {
    pub turn_id: TurnId,
    pub thread_id: ThreadId,
}

#[derive(Clone)]
pub struct ThreadHandle {
    pub thread_id: ThreadId,
    pub(in crate::runtime::api) runtime: Runtime,
}

impl std::fmt::Debug for ThreadHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ThreadHandle")
            .field("thread_id", &self.thread_id)
            .finish()
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ThreadStartParams {
    pub model: Option<String>,
    pub cwd: Option<String>,
    pub approval_policy: Option<ApprovalPolicy>,
    pub sandbox_policy: Option<SandboxPolicy>,
    pub privileged_escalation_approved: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadReadParams {
    pub thread_id: ThreadId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_turns: Option<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThreadListSortKey {
    #[serde(rename = "created_at")]
    CreatedAt,
    #[serde(rename = "updated_at")]
    UpdatedAt,
}

impl ThreadListSortKey {
    pub fn as_wire(self) -> &'static str {
        match self {
            Self::CreatedAt => "created_at",
            Self::UpdatedAt => "updated_at",
        }
    }
}

impl FromStr for ThreadListSortKey {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "created_at" => Ok(Self::CreatedAt),
            "updated_at" => Ok(Self::UpdatedAt),
            other => Err(format!("unknown thread list sort key: {other}")),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadListParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_providers: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort_key: Option<ThreadListSortKey>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadListResponse {
    pub data: Vec<ThreadView>,
    #[serde(default)]
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadLoadedListParams {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadLoadedListResponse {
    pub data: Vec<String>,
    #[serde(default)]
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadRollbackParams {
    pub thread_id: ThreadId,
    pub num_turns: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ThreadRollbackResponse {
    pub thread: ThreadView,
}
