use std::str::FromStr;
use std::time::Duration;

use coclai_plugin_core::HookPhase;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};
use tokio::sync::broadcast::error::RecvError;
use tokio::time::{timeout, Instant};

use crate::errors::RpcError;
use crate::errors::RuntimeError;
use crate::hooks::{PreHookDecision, RuntimeHookConfig};
use crate::runtime::Runtime;
use crate::turn_output::{parse_thread_id, AssistantTextCollector};

mod flow;
mod models;
mod ops;
mod turn_error;
mod wire;

use flow::{
    apply_pre_hook_actions_to_prompt, apply_pre_hook_actions_to_session, build_hook_context,
    extract_assistant_text_from_turn, interrupt_turn_best_effort, result_status, HookContextInput,
    HookExecutionState, LaggedTurnTerminal, PromptMutationState, SessionMutationState,
};
pub use models::{
    PromptRunError, PromptRunParams, PromptRunResult, PromptTurnFailure, PromptTurnTerminalState,
};
use ops::{deserialize_result, serialize_params};
use turn_error::{extract_turn_error_signal, PromptTurnErrorSignal};
use wire::{
    build_prompt_inputs, thread_overrides_to_wire, thread_start_params_to_wire,
    validate_prompt_attachments, validate_thread_start_security,
};
#[cfg(test)]
use wire::{input_item_to_wire, turn_start_params_to_wire};

pub type RpcId = u64;
pub type ThreadId = String;
pub type TurnId = String;
pub type ItemId = String;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InputItem {
    Text {
        text: String,
    },
    TextWithElements {
        text: String,
        text_elements: Vec<TextElement>,
    },
    ImageUrl {
        url: String,
    },
    LocalImage {
        path: String,
    },
    Skill {
        name: String,
        path: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ByteRange {
    pub start: u64,
    pub end: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextElement {
    pub byte_range: ByteRange,
    pub placeholder: Option<String>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ApprovalPolicy {
    #[serde(rename = "untrusted")]
    Untrusted,
    #[serde(rename = "on-failure")]
    OnFailure,
    #[serde(rename = "on-request")]
    OnRequest,
    #[serde(rename = "never")]
    Never,
}

impl ApprovalPolicy {
    pub fn as_wire(self) -> &'static str {
        match self {
            Self::Untrusted => "untrusted",
            Self::OnFailure => "on-failure",
            Self::OnRequest => "on-request",
            Self::Never => "never",
        }
    }
}

impl FromStr for ApprovalPolicy {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "untrusted" => Ok(Self::Untrusted),
            "on-failure" => Ok(Self::OnFailure),
            "on-request" => Ok(Self::OnRequest),
            "never" => Ok(Self::Never),
            other => Err(format!("unknown approval policy: {other}")),
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum ReasoningEffort {
    #[serde(rename = "none")]
    None,
    #[serde(rename = "minimal")]
    Minimal,
    #[serde(rename = "low")]
    Low,
    #[serde(rename = "medium")]
    Medium,
    #[serde(rename = "high")]
    High,
    #[serde(rename = "xhigh")]
    XHigh,
}

impl ReasoningEffort {
    pub fn as_wire(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::XHigh => "xhigh",
        }
    }
}

/// Deterministic default effort used when callers do not provide one.
/// Chosen for broad model compatibility while keeping reasoning enabled.
pub const DEFAULT_REASONING_EFFORT: ReasoningEffort = ReasoningEffort::Medium;

impl FromStr for ReasoningEffort {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "none" => Ok(Self::None),
            "minimal" => Ok(Self::Minimal),
            "low" => Ok(Self::Low),
            "medium" => Ok(Self::Medium),
            "high" => Ok(Self::High),
            "xhigh" => Ok(Self::XHigh),
            other => Err(format!("unknown reasoning effort: {other}")),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PromptAttachment {
    AtPath {
        path: String,
        placeholder: Option<String>,
    },
    ImageUrl {
        url: String,
    },
    LocalImage {
        path: String,
    },
    Skill {
        name: String,
        path: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExternalNetworkAccess {
    Restricted,
    Enabled,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SandboxPreset {
    ReadOnly,
    WorkspaceWrite {
        writable_roots: Vec<String>,
        network_access: bool,
    },
    DangerFullAccess,
    ExternalSandbox {
        network_access: ExternalNetworkAccess,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum SandboxPolicy {
    Preset(SandboxPreset),
    Raw(Value),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ThreadStartParams {
    pub model: Option<String>,
    pub cwd: Option<String>,
    pub approval_policy: Option<ApprovalPolicy>,
    pub sandbox_policy: Option<SandboxPolicy>,
    /// Explicit opt-in gate for privileged sandbox usage (SEC-004).
    pub privileged_escalation_approved: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadReadParams {
    pub thread_id: ThreadId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_turns: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadView {
    pub id: ThreadId,
    pub cli_version: String,
    pub created_at: i64,
    pub cwd: String,
    #[serde(default)]
    pub git_info: Option<Value>,
    pub model_provider: String,
    pub path: String,
    pub preview: String,
    pub source: String,
    pub turns: Vec<ThreadTurnView>,
    pub updated_at: i64,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadTurnView {
    pub id: TurnId,
    pub status: ThreadTurnStatus,
    #[serde(default)]
    pub items: Vec<ThreadItemView>,
    #[serde(default)]
    pub error: Option<ThreadTurnErrorView>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThreadTurnStatus {
    #[serde(rename = "completed")]
    Completed,
    #[serde(rename = "interrupted")]
    Interrupted,
    #[serde(rename = "failed")]
    Failed,
    #[serde(rename = "inProgress")]
    InProgress,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadTurnErrorView {
    pub message: String,
    #[serde(default)]
    pub additional_details: Option<String>,
    #[serde(default)]
    pub codex_error_info: Option<Value>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ThreadItemType {
    UserMessage,
    AgentMessage,
    Reasoning,
    CommandExecution,
    FileChange,
    McpToolCall,
    CollabAgentToolCall,
    WebSearch,
    ImageView,
    EnteredReviewMode,
    ExitedReviewMode,
    Unknown(String),
}

impl ThreadItemType {
    pub fn as_wire(&self) -> &str {
        match self {
            Self::UserMessage => "userMessage",
            Self::AgentMessage => "agentMessage",
            Self::Reasoning => "reasoning",
            Self::CommandExecution => "commandExecution",
            Self::FileChange => "fileChange",
            Self::McpToolCall => "mcpToolCall",
            Self::CollabAgentToolCall => "collabAgentToolCall",
            Self::WebSearch => "webSearch",
            Self::ImageView => "imageView",
            Self::EnteredReviewMode => "enteredReviewMode",
            Self::ExitedReviewMode => "exitedReviewMode",
            Self::Unknown(raw) => raw.as_str(),
        }
    }

    pub fn from_wire(raw: &str) -> Self {
        match raw {
            "userMessage" => Self::UserMessage,
            "agentMessage" => Self::AgentMessage,
            "reasoning" => Self::Reasoning,
            "commandExecution" => Self::CommandExecution,
            "fileChange" => Self::FileChange,
            "mcpToolCall" => Self::McpToolCall,
            "collabAgentToolCall" => Self::CollabAgentToolCall,
            "webSearch" => Self::WebSearch,
            "imageView" => Self::ImageView,
            "enteredReviewMode" => Self::EnteredReviewMode,
            "exitedReviewMode" => Self::ExitedReviewMode,
            _ => Self::Unknown(raw.to_owned()),
        }
    }
}

impl Serialize for ThreadItemType {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_wire())
    }
}

impl<'de> Deserialize<'de> for ThreadItemType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Ok(Self::from_wire(raw.as_str()))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadAgentMessageItemView {
    pub text: String,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadCommandExecutionItemView {
    pub command: String,
    pub command_actions: Vec<Value>,
    pub cwd: String,
    pub status: String,
    #[serde(default)]
    pub aggregated_output: Option<String>,
    #[serde(default)]
    pub duration_ms: Option<i64>,
    #[serde(default)]
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub process_id: Option<String>,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ThreadItemPayloadView {
    AgentMessage(ThreadAgentMessageItemView),
    CommandExecution(ThreadCommandExecutionItemView),
    Unknown(Map<String, Value>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ThreadItemView {
    pub id: ItemId,
    pub item_type: ThreadItemType,
    pub payload: ThreadItemPayloadView,
}

impl Serialize for ThreadItemView {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let object = match &self.payload {
            ThreadItemPayloadView::AgentMessage(data) => {
                serde_json::to_value(data).map_err(serde::ser::Error::custom)?
            }
            ThreadItemPayloadView::CommandExecution(data) => {
                serde_json::to_value(data).map_err(serde::ser::Error::custom)?
            }
            ThreadItemPayloadView::Unknown(extra) => Value::Object(extra.clone()),
        };
        let Value::Object(mut fields) = object else {
            return Err(serde::ser::Error::custom(
                "thread item payload must serialize to object",
            ));
        };

        fields.remove("id");
        fields.remove("type");
        fields.insert("id".to_owned(), Value::String(self.id.clone()));
        fields.insert(
            "type".to_owned(),
            Value::String(self.item_type.as_wire().to_owned()),
        );
        Value::Object(fields).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for ThreadItemView {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let mut fields = Map::<String, Value>::deserialize(deserializer)?;
        let id = fields
            .remove("id")
            .and_then(|v| v.as_str().map(ToOwned::to_owned))
            .ok_or_else(|| serde::de::Error::custom("thread item missing string id"))?;
        let raw_type = fields
            .remove("type")
            .and_then(|v| v.as_str().map(ToOwned::to_owned))
            .ok_or_else(|| serde::de::Error::custom("thread item missing string type"))?;
        let item_type = ThreadItemType::from_wire(raw_type.as_str());

        let payload = match &item_type {
            ThreadItemType::AgentMessage => {
                let data: ThreadAgentMessageItemView =
                    serde_json::from_value(Value::Object(fields))
                        .map_err(serde::de::Error::custom)?;
                ThreadItemPayloadView::AgentMessage(data)
            }
            ThreadItemType::CommandExecution => {
                let data: ThreadCommandExecutionItemView =
                    serde_json::from_value(Value::Object(fields))
                        .map_err(serde::de::Error::custom)?;
                ThreadItemPayloadView::CommandExecution(data)
            }
            _ => ThreadItemPayloadView::Unknown(fields),
        };

        Ok(Self {
            id,
            item_type,
            payload,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ThreadReadResponse {
    pub thread: ThreadView,
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

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TurnStartParams {
    pub input: Vec<InputItem>,
    pub cwd: Option<String>,
    pub approval_policy: Option<ApprovalPolicy>,
    pub sandbox_policy: Option<SandboxPolicy>,
    /// Explicit opt-in gate for privileged sandbox usage (SEC-004).
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
    runtime: Runtime,
}

impl std::fmt::Debug for ThreadHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ThreadHandle")
            .field("thread_id", &self.thread_id)
            .finish()
    }
}

impl Runtime {
    /// Run one prompt with safe default policies using only cwd + prompt.
    /// Side effects: same as `run_prompt`. Allocation: params object + two Strings.
    /// Complexity: O(n), n = input string lengths + streamed turn output size.
    pub async fn run_prompt_simple(
        &self,
        cwd: impl Into<String>,
        prompt: impl Into<String>,
    ) -> Result<PromptRunResult, PromptRunError> {
        self.run_prompt(PromptRunParams::new(cwd, prompt)).await
    }

    pub async fn thread_start(&self, p: ThreadStartParams) -> Result<ThreadHandle, RpcError> {
        self.thread_start_with_hooks(p, None).await
    }

    pub(crate) async fn thread_start_with_hooks(
        &self,
        p: ThreadStartParams,
        scoped_hooks: Option<&RuntimeHookConfig>,
    ) -> Result<ThreadHandle, RpcError> {
        if !self.hooks_enabled_with(scoped_hooks) {
            return self.thread_start_raw(p).await;
        }

        let mut hook_state = HookExecutionState::new(self.next_hook_correlation_id());
        let mut session_state =
            SessionMutationState::from_thread_start(&p, hook_state.metadata.clone());
        let decisions = self
            .execute_pre_hook_phase(
                &mut hook_state,
                HookPhase::PreSessionStart,
                p.cwd.as_deref(),
                p.model.as_deref(),
                None,
                None,
                scoped_hooks,
            )
            .await;
        apply_pre_hook_actions_to_session(
            &mut session_state,
            HookPhase::PreSessionStart,
            decisions,
            &mut hook_state.report,
        );
        hook_state.metadata = session_state.metadata.clone();
        let mut p = p;
        p.model = session_state.model;

        let start_cwd = p.cwd.clone();
        let start_model = p.model.clone();
        let result = self.thread_start_raw(p).await;
        let post_thread_id = result.as_ref().ok().map(|thread| thread.thread_id.as_str());
        self.execute_post_hook_phase(
            &mut hook_state,
            HookContextInput {
                phase: HookPhase::PostSessionStart,
                cwd: start_cwd.as_deref(),
                model: start_model.as_deref(),
                thread_id: post_thread_id,
                turn_id: None,
                main_status: Some(result_status(&result)),
            },
            scoped_hooks,
        )
        .await;
        self.publish_hook_report(hook_state.report);
        result
    }

    async fn thread_start_raw(&self, p: ThreadStartParams) -> Result<ThreadHandle, RpcError> {
        validate_thread_start_security(&p)?;
        let response = self
            .call_raw("thread/start", thread_start_params_to_wire(&p))
            .await?;
        let thread_id = parse_thread_id(&response).ok_or_else(|| {
            RpcError::InvalidRequest(format!(
                "thread/start missing thread id in result: {response}"
            ))
        })?;
        Ok(ThreadHandle {
            thread_id,
            runtime: self.clone(),
        })
    }

    pub async fn thread_resume(
        &self,
        thread_id: &str,
        p: ThreadStartParams,
    ) -> Result<ThreadHandle, RpcError> {
        self.thread_resume_with_hooks(thread_id, p, None).await
    }

    pub(crate) async fn thread_resume_with_hooks(
        &self,
        thread_id: &str,
        p: ThreadStartParams,
        scoped_hooks: Option<&RuntimeHookConfig>,
    ) -> Result<ThreadHandle, RpcError> {
        if !self.hooks_enabled_with(scoped_hooks) {
            return self.thread_resume_raw(thread_id, p).await;
        }

        let mut hook_state = HookExecutionState::new(self.next_hook_correlation_id());
        let mut session_state =
            SessionMutationState::from_thread_start(&p, hook_state.metadata.clone());
        let decisions = self
            .execute_pre_hook_phase(
                &mut hook_state,
                HookPhase::PreSessionStart,
                p.cwd.as_deref(),
                p.model.as_deref(),
                Some(thread_id),
                None,
                scoped_hooks,
            )
            .await;
        apply_pre_hook_actions_to_session(
            &mut session_state,
            HookPhase::PreSessionStart,
            decisions,
            &mut hook_state.report,
        );
        hook_state.metadata = session_state.metadata.clone();
        let mut p = p;
        p.model = session_state.model;

        let resume_cwd = p.cwd.clone();
        let resume_model = p.model.clone();
        let result = self.thread_resume_raw(thread_id, p).await;
        let post_thread_id = result
            .as_ref()
            .ok()
            .map(|thread| thread.thread_id.as_str())
            .or(Some(thread_id));
        self.execute_post_hook_phase(
            &mut hook_state,
            HookContextInput {
                phase: HookPhase::PostSessionStart,
                cwd: resume_cwd.as_deref(),
                model: resume_model.as_deref(),
                thread_id: post_thread_id,
                turn_id: None,
                main_status: Some(result_status(&result)),
            },
            scoped_hooks,
        )
        .await;
        self.publish_hook_report(hook_state.report);
        result
    }

    async fn thread_resume_raw(
        &self,
        thread_id: &str,
        p: ThreadStartParams,
    ) -> Result<ThreadHandle, RpcError> {
        validate_thread_start_security(&p)?;
        let mut params = Map::<String, Value>::new();
        params.insert("threadId".to_owned(), Value::String(thread_id.to_owned()));
        let overrides = thread_overrides_to_wire(&p);
        if !overrides.is_empty() {
            params.insert("overrides".to_owned(), Value::Object(overrides));
        }

        let response = self
            .call_raw("thread/resume", Value::Object(params))
            .await?;
        let resumed = parse_thread_id(&response).ok_or_else(|| {
            RpcError::InvalidRequest(format!(
                "thread/resume missing thread id in result: {response}"
            ))
        })?;
        if resumed != thread_id {
            return Err(RpcError::InvalidRequest(format!(
                "thread/resume returned mismatched thread id: requested={thread_id} actual={resumed}"
            )));
        }
        Ok(ThreadHandle {
            thread_id: resumed,
            runtime: self.clone(),
        })
    }

    pub async fn thread_fork(&self, thread_id: &str) -> Result<ThreadHandle, RpcError> {
        let mut params = Map::<String, Value>::new();
        params.insert("threadId".to_owned(), Value::String(thread_id.to_owned()));
        let response = self.call_raw("thread/fork", Value::Object(params)).await?;
        let forked = parse_thread_id(&response).ok_or_else(|| {
            RpcError::InvalidRequest(format!(
                "thread/fork missing thread id in result: {response}"
            ))
        })?;
        Ok(ThreadHandle {
            thread_id: forked,
            runtime: self.clone(),
        })
    }

    /// Archive a thread (logical close on server side).
    /// Allocation: one JSON object with thread id.
    /// Complexity: O(1).
    pub async fn thread_archive(&self, thread_id: &str) -> Result<(), RpcError> {
        let mut params = Map::<String, Value>::new();
        params.insert("threadId".to_owned(), Value::String(thread_id.to_owned()));
        let _ = self
            .call_raw("thread/archive", Value::Object(params))
            .await?;
        Ok(())
    }

    /// Read one thread by id.
    /// Allocation: serialized params + decoded response object.
    /// Complexity: O(n), n = thread payload size.
    pub async fn thread_read(&self, p: ThreadReadParams) -> Result<ThreadReadResponse, RpcError> {
        let params = serialize_params("thread/read", &p)?;
        let response = self.call_raw("thread/read", params).await?;
        deserialize_result("thread/read", response)
    }

    /// List persisted threads with optional filters and pagination.
    /// Allocation: serialized params + decoded list payload.
    /// Complexity: O(n), n = number of returned threads.
    pub async fn thread_list(&self, p: ThreadListParams) -> Result<ThreadListResponse, RpcError> {
        let params = serialize_params("thread/list", &p)?;
        let response = self.call_raw("thread/list", params).await?;
        deserialize_result("thread/list", response)
    }

    /// List currently loaded thread ids from in-memory sessions.
    /// Allocation: serialized params + decoded list payload.
    /// Complexity: O(n), n = number of returned ids.
    pub async fn thread_loaded_list(
        &self,
        p: ThreadLoadedListParams,
    ) -> Result<ThreadLoadedListResponse, RpcError> {
        let params = serialize_params("thread/loaded/list", &p)?;
        let response = self.call_raw("thread/loaded/list", params).await?;
        deserialize_result("thread/loaded/list", response)
    }

    /// Roll back the last `num_turns` turns from a thread.
    /// Allocation: serialized params + decoded response payload.
    /// Complexity: O(n), n = rolled thread payload size.
    pub async fn thread_rollback(
        &self,
        p: ThreadRollbackParams,
    ) -> Result<ThreadRollbackResponse, RpcError> {
        let params = serialize_params("thread/rollback", &p)?;
        let response = self.call_raw("thread/rollback", params).await?;
        deserialize_result("thread/rollback", response)
    }

    /// Interrupt one in-flight turn for a thread.
    /// Allocation: one JSON object with thread + turn id.
    /// Complexity: O(1).
    pub async fn turn_interrupt(&self, thread_id: &str, turn_id: &str) -> Result<(), RpcError> {
        let mut params = Map::<String, Value>::new();
        params.insert("threadId".to_owned(), Value::String(thread_id.to_owned()));
        params.insert("turnId".to_owned(), Value::String(turn_id.to_owned()));
        let _ = self
            .call_raw("turn/interrupt", Value::Object(params))
            .await?;
        Ok(())
    }

    /// Interrupt one in-flight turn with explicit RPC timeout.
    /// Allocation: one JSON object with thread + turn id.
    /// Complexity: O(1).
    pub async fn turn_interrupt_with_timeout(
        &self,
        thread_id: &str,
        turn_id: &str,
        timeout_duration: Duration,
    ) -> Result<(), RpcError> {
        let mut params = Map::<String, Value>::new();
        params.insert("threadId".to_owned(), Value::String(thread_id.to_owned()));
        params.insert("turnId".to_owned(), Value::String(turn_id.to_owned()));
        let _ = self
            .call_raw_with_timeout("turn/interrupt", Value::Object(params), timeout_duration)
            .await?;
        Ok(())
    }

    /// Run one prompt end-to-end and return the final assistant text.
    /// Side effects: sends thread/turn RPC calls and consumes live event stream.
    /// Allocation: O(n), n = prompt length + attachment count + streamed text.
    pub async fn run_prompt(&self, p: PromptRunParams) -> Result<PromptRunResult, PromptRunError> {
        self.run_prompt_with_hooks(p, None).await
    }

    pub(crate) async fn run_prompt_with_hooks(
        &self,
        p: PromptRunParams,
        scoped_hooks: Option<&RuntimeHookConfig>,
    ) -> Result<PromptRunResult, PromptRunError> {
        if !self.hooks_enabled_with(scoped_hooks) {
            validate_prompt_attachments(&p.cwd, &p.attachments)?;
            let effort = p.effort.unwrap_or(DEFAULT_REASONING_EFFORT);
            let thread = self
                .thread_start_raw(ThreadStartParams {
                    model: p.model.clone(),
                    cwd: Some(p.cwd.clone()),
                    approval_policy: Some(p.approval_policy),
                    sandbox_policy: Some(p.sandbox_policy.clone()),
                    privileged_escalation_approved: p.privileged_escalation_approved,
                })
                .await?;
            return self
                .run_prompt_on_thread(thread, p, effort, None, scoped_hooks)
                .await;
        }

        let mut p = p;
        let mut hook_state = HookExecutionState::new(self.next_hook_correlation_id());
        let mut prompt_state = PromptMutationState::from_params(&p, hook_state.metadata.clone());
        let decisions = self
            .execute_pre_hook_phase(
                &mut hook_state,
                HookPhase::PreRun,
                Some(prompt_state.prompt.as_str()),
                prompt_state.model.as_deref(),
                None,
                None,
                scoped_hooks,
            )
            .await;
        apply_pre_hook_actions_to_prompt(
            &mut prompt_state,
            p.cwd.as_str(),
            HookPhase::PreRun,
            decisions,
            &mut hook_state.report,
        );
        hook_state.metadata = prompt_state.metadata.clone();
        p.prompt = prompt_state.prompt;
        p.model = prompt_state.model;
        p.attachments = prompt_state.attachments;
        let run_cwd = p.cwd.clone();
        let run_model = p.model.clone();

        let result = match validate_prompt_attachments(&p.cwd, &p.attachments) {
            Ok(()) => {
                let effort = p.effort.unwrap_or(DEFAULT_REASONING_EFFORT);
                let thread = self
                    .thread_start_raw(ThreadStartParams {
                        model: p.model.clone(),
                        cwd: Some(p.cwd.clone()),
                        approval_policy: Some(p.approval_policy),
                        sandbox_policy: Some(p.sandbox_policy.clone()),
                        privileged_escalation_approved: p.privileged_escalation_approved,
                    })
                    .await?;
                self.run_prompt_on_thread(thread, p, effort, Some(&mut hook_state), scoped_hooks)
                    .await
            }
            Err(err) => Err(err),
        };

        let post_thread_id = result.as_ref().ok().map(|value| value.thread_id.as_str());
        self.execute_post_hook_phase(
            &mut hook_state,
            HookContextInput {
                phase: HookPhase::PostRun,
                cwd: Some(run_cwd.as_str()),
                model: run_model.as_deref(),
                thread_id: post_thread_id,
                turn_id: None,
                main_status: Some(result_status(&result)),
            },
            scoped_hooks,
        )
        .await;
        self.publish_hook_report(hook_state.report);
        result
    }

    /// Continue an existing thread with one additional prompt turn.
    /// Side effects: sends thread/resume + turn/start RPC calls and consumes live event stream.
    /// Allocation: O(n), n = prompt length + attachment count + streamed text.
    pub async fn run_prompt_in_thread(
        &self,
        thread_id: &str,
        p: PromptRunParams,
    ) -> Result<PromptRunResult, PromptRunError> {
        self.run_prompt_in_thread_with_hooks(thread_id, p, None)
            .await
    }

    pub(crate) async fn run_prompt_in_thread_with_hooks(
        &self,
        thread_id: &str,
        p: PromptRunParams,
        scoped_hooks: Option<&RuntimeHookConfig>,
    ) -> Result<PromptRunResult, PromptRunError> {
        if !self.hooks_enabled_with(scoped_hooks) {
            validate_prompt_attachments(&p.cwd, &p.attachments)?;
            let effort = p.effort.unwrap_or(DEFAULT_REASONING_EFFORT);
            let thread = self
                .thread_resume_raw(
                    thread_id,
                    ThreadStartParams {
                        model: p.model.clone(),
                        cwd: Some(p.cwd.clone()),
                        approval_policy: Some(p.approval_policy),
                        sandbox_policy: Some(p.sandbox_policy.clone()),
                        privileged_escalation_approved: p.privileged_escalation_approved,
                    },
                )
                .await?;
            return self
                .run_prompt_on_thread(thread, p, effort, None, scoped_hooks)
                .await;
        }

        let mut p = p;
        let mut hook_state = HookExecutionState::new(self.next_hook_correlation_id());
        let mut prompt_state = PromptMutationState::from_params(&p, hook_state.metadata.clone());
        let decisions = self
            .execute_pre_hook_phase(
                &mut hook_state,
                HookPhase::PreRun,
                Some(prompt_state.prompt.as_str()),
                prompt_state.model.as_deref(),
                Some(thread_id),
                None,
                scoped_hooks,
            )
            .await;
        apply_pre_hook_actions_to_prompt(
            &mut prompt_state,
            p.cwd.as_str(),
            HookPhase::PreRun,
            decisions,
            &mut hook_state.report,
        );
        hook_state.metadata = prompt_state.metadata.clone();
        p.prompt = prompt_state.prompt;
        p.model = prompt_state.model;
        p.attachments = prompt_state.attachments;
        let run_cwd = p.cwd.clone();
        let run_model = p.model.clone();

        let result = match validate_prompt_attachments(&p.cwd, &p.attachments) {
            Ok(()) => {
                let effort = p.effort.unwrap_or(DEFAULT_REASONING_EFFORT);
                let thread = self
                    .thread_resume_raw(
                        thread_id,
                        ThreadStartParams {
                            model: p.model.clone(),
                            cwd: Some(p.cwd.clone()),
                            approval_policy: Some(p.approval_policy),
                            sandbox_policy: Some(p.sandbox_policy.clone()),
                            privileged_escalation_approved: p.privileged_escalation_approved,
                        },
                    )
                    .await?;
                self.run_prompt_on_thread(thread, p, effort, Some(&mut hook_state), scoped_hooks)
                    .await
            }
            Err(err) => Err(err),
        };

        let post_thread_id = result
            .as_ref()
            .ok()
            .map(|value| value.thread_id.as_str())
            .or(Some(thread_id));
        self.execute_post_hook_phase(
            &mut hook_state,
            HookContextInput {
                phase: HookPhase::PostRun,
                cwd: Some(run_cwd.as_str()),
                model: run_model.as_deref(),
                thread_id: post_thread_id,
                turn_id: None,
                main_status: Some(result_status(&result)),
            },
            scoped_hooks,
        )
        .await;
        self.publish_hook_report(hook_state.report);
        result
    }

    async fn run_prompt_on_thread(
        &self,
        thread: ThreadHandle,
        p: PromptRunParams,
        effort: ReasoningEffort,
        hook_state: Option<&mut HookExecutionState>,
        scoped_hooks: Option<&RuntimeHookConfig>,
    ) -> Result<PromptRunResult, PromptRunError> {
        let mut hook_state = hook_state;
        let mut p = p;
        if let Some(state) = hook_state.as_deref_mut() {
            let mut prompt_state = PromptMutationState::from_params(&p, state.metadata.clone());
            let decisions = self
                .execute_pre_hook_phase(
                    state,
                    HookPhase::PreTurn,
                    Some(prompt_state.prompt.as_str()),
                    prompt_state.model.as_deref(),
                    Some(thread.thread_id.as_str()),
                    None,
                    scoped_hooks,
                )
                .await;
            apply_pre_hook_actions_to_prompt(
                &mut prompt_state,
                p.cwd.as_str(),
                HookPhase::PreTurn,
                decisions,
                &mut state.report,
            );
            state.metadata = prompt_state.metadata;
            p.prompt = prompt_state.prompt;
            p.model = prompt_state.model;
            p.attachments = prompt_state.attachments;
        }

        let mut live_rx = self.subscribe_live();
        let mut post_turn_id: Option<String> = None;
        let model = p.model.clone();
        let run_result = match thread
            .turn_start(TurnStartParams {
                input: build_prompt_inputs(&p.prompt, &p.attachments),
                cwd: Some(p.cwd.clone()),
                approval_policy: Some(p.approval_policy),
                sandbox_policy: Some(p.sandbox_policy),
                privileged_escalation_approved: p.privileged_escalation_approved,
                model,
                effort: Some(effort),
                summary: None,
                output_schema: None,
            })
            .await
        {
            Ok(turn) => {
                post_turn_id = Some(turn.turn_id.clone());
                let mut collector = AssistantTextCollector::new();
                let mut last_turn_error: Option<PromptTurnErrorSignal> = None;
                let mut lagged_completed_text: Option<String> = None;
                let deadline = Instant::now() + p.timeout;
                let terminal = loop {
                    let now = Instant::now();
                    if now >= deadline {
                        interrupt_turn_best_effort(&thread, &turn.turn_id);
                        break Err(PromptRunError::Timeout(p.timeout));
                    }
                    let remaining = deadline.saturating_duration_since(now);

                    let envelope = match timeout(remaining, live_rx.recv()).await {
                        Ok(Ok(v)) => v,
                        Ok(Err(RecvError::Lagged(_))) => {
                            match self
                                .read_turn_terminal_after_lag(&thread.thread_id, &turn.turn_id)
                                .await
                            {
                                Ok(Some(LaggedTurnTerminal::Completed { assistant_text })) => {
                                    lagged_completed_text = assistant_text;
                                    break Ok(());
                                }
                                Ok(Some(LaggedTurnTerminal::Failed { message })) => {
                                    if let Some(err) = last_turn_error.clone() {
                                        break Err(PromptRunError::TurnFailedWithContext(
                                            err.into_failure(PromptTurnTerminalState::Failed),
                                        ));
                                    }
                                    if let Some(message) = message {
                                        break Err(PromptRunError::TurnFailedWithContext(
                                            PromptTurnFailure {
                                                terminal_state: PromptTurnTerminalState::Failed,
                                                source_method: "thread/read".to_owned(),
                                                code: None,
                                                message,
                                            },
                                        ));
                                    }
                                    break Err(PromptRunError::TurnFailed);
                                }
                                Ok(Some(LaggedTurnTerminal::Interrupted)) => {
                                    break Err(PromptRunError::TurnInterrupted);
                                }
                                Ok(None) => continue,
                                Err(err) => break Err(PromptRunError::Rpc(err)),
                            }
                        }
                        Ok(Err(RecvError::Closed)) => {
                            break Err(PromptRunError::Runtime(RuntimeError::Internal(format!(
                                "live stream closed: {}",
                                RecvError::Closed
                            ))));
                        }
                        Err(_) => {
                            interrupt_turn_best_effort(&thread, &turn.turn_id);
                            break Err(PromptRunError::Timeout(p.timeout));
                        }
                    };

                    if envelope.thread_id.as_deref() != Some(&thread.thread_id) {
                        continue;
                    }
                    if envelope.turn_id.as_deref() != Some(&turn.turn_id) {
                        continue;
                    }

                    collector.push_envelope(&envelope);
                    if let Some(err) = extract_turn_error_signal(&envelope) {
                        last_turn_error = Some(err);
                    }

                    match envelope.method.as_deref() {
                        Some("turn/completed") => break Ok(()),
                        Some("turn/failed") => {
                            if let Some(err) = last_turn_error.clone() {
                                break Err(PromptRunError::TurnFailedWithContext(
                                    err.into_failure(PromptTurnTerminalState::Failed),
                                ));
                            }
                            break Err(PromptRunError::TurnFailed);
                        }
                        Some("turn/interrupted") => break Err(PromptRunError::TurnInterrupted),
                        _ => {}
                    }
                };

                match terminal {
                    Err(err) => Err(err),
                    Ok(()) => {
                        let assistant_text = if let Some(snapshot_text) = lagged_completed_text {
                            if snapshot_text.trim().is_empty() {
                                collector.into_text()
                            } else {
                                snapshot_text
                            }
                        } else {
                            collector.into_text()
                        };
                        let assistant_text = assistant_text.trim().to_owned();
                        if assistant_text.is_empty() {
                            if let Some(err) = last_turn_error {
                                Err(PromptRunError::TurnCompletedWithoutAssistantText(
                                    err.into_failure(
                                        PromptTurnTerminalState::CompletedWithoutAssistantText,
                                    ),
                                ))
                            } else {
                                Err(PromptRunError::EmptyAssistantText)
                            }
                        } else {
                            Ok(PromptRunResult {
                                thread_id: thread.thread_id.clone(),
                                turn_id: turn.turn_id,
                                assistant_text,
                            })
                        }
                    }
                }
            }
            Err(err) => Err(PromptRunError::Rpc(err)),
        };

        if let Some(state) = hook_state {
            self.execute_post_hook_phase(
                state,
                HookContextInput {
                    phase: HookPhase::PostTurn,
                    cwd: Some(p.cwd.as_str()),
                    model: p.model.as_deref(),
                    thread_id: Some(thread.thread_id.as_str()),
                    turn_id: post_turn_id.as_deref(),
                    main_status: Some(result_status(&run_result)),
                },
                scoped_hooks,
            )
            .await;
        }

        run_result
    }

    async fn read_turn_terminal_after_lag(
        &self,
        thread_id: &str,
        turn_id: &str,
    ) -> Result<Option<LaggedTurnTerminal>, RpcError> {
        let response = self
            .thread_read(ThreadReadParams {
                thread_id: thread_id.to_owned(),
                include_turns: Some(true),
            })
            .await?;

        let Some(turn) = response.thread.turns.iter().find(|turn| turn.id == turn_id) else {
            return Ok(None);
        };

        let terminal = match turn.status {
            ThreadTurnStatus::Completed => Some(LaggedTurnTerminal::Completed {
                assistant_text: extract_assistant_text_from_turn(turn),
            }),
            ThreadTurnStatus::Failed => Some(LaggedTurnTerminal::Failed {
                message: turn.error.as_ref().map(|error| error.message.clone()),
            }),
            ThreadTurnStatus::Interrupted => Some(LaggedTurnTerminal::Interrupted),
            ThreadTurnStatus::InProgress => None,
        };
        Ok(terminal)
    }

    #[allow(clippy::too_many_arguments)]
    async fn execute_pre_hook_phase(
        &self,
        hook_state: &mut HookExecutionState,
        phase: HookPhase,
        cwd: Option<&str>,
        model: Option<&str>,
        thread_id: Option<&str>,
        turn_id: Option<&str>,
        scoped_hooks: Option<&RuntimeHookConfig>,
    ) -> Vec<PreHookDecision> {
        let ctx = build_hook_context(
            hook_state.correlation_id.as_str(),
            &hook_state.metadata,
            HookContextInput {
                phase,
                cwd,
                model,
                thread_id,
                turn_id,
                main_status: None,
            },
        );
        self.run_pre_hooks_with(&ctx, &mut hook_state.report, scoped_hooks)
            .await
    }

    async fn execute_post_hook_phase(
        &self,
        hook_state: &mut HookExecutionState,
        input: HookContextInput<'_>,
        scoped_hooks: Option<&RuntimeHookConfig>,
    ) {
        let ctx = build_hook_context(
            hook_state.correlation_id.as_str(),
            &hook_state.metadata,
            input,
        );
        self.run_post_hooks_with(&ctx, &mut hook_state.report, scoped_hooks)
            .await;
    }
}

#[cfg(test)]
mod tests;
