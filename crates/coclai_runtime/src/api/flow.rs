use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use coclai_plugin_core::{
    HookAction, HookAttachment, HookContext, HookIssue, HookIssueClass, HookPatch, HookPhase,
    HookReport,
};
use serde_json::{Map, Value};

use crate::hooks::PreHookDecision;

use super::{
    PromptAttachment, PromptRunParams, ThreadHandle, ThreadItemPayloadView, ThreadStartParams,
    ThreadTurnView,
};

#[derive(Clone, Debug)]
pub(super) struct HookExecutionState {
    pub(super) correlation_id: String,
    pub(super) report: HookReport,
    pub(super) metadata: Value,
}

impl HookExecutionState {
    pub(super) fn new(correlation_id: String) -> Self {
        Self {
            correlation_id,
            report: HookReport::default(),
            metadata: Value::Object(Map::new()),
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct PromptMutationState {
    pub(super) prompt: String,
    pub(super) model: Option<String>,
    pub(super) attachments: Vec<PromptAttachment>,
    pub(super) metadata: Value,
}

impl PromptMutationState {
    pub(super) fn from_params(p: &PromptRunParams, metadata: Value) -> Self {
        Self {
            prompt: p.prompt.clone(),
            model: p.model.clone(),
            attachments: p.attachments.clone(),
            metadata,
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct SessionMutationState {
    pub(super) model: Option<String>,
    pub(super) metadata: Value,
}

impl SessionMutationState {
    pub(super) fn from_thread_start(p: &ThreadStartParams, metadata: Value) -> Self {
        Self {
            model: p.model.clone(),
            metadata,
        }
    }
}

fn now_millis_for_hook_context() -> i64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_millis() as i64,
        Err(_) => 0,
    }
}

#[derive(Clone, Copy)]
pub(super) struct HookContextInput<'a> {
    pub(super) phase: HookPhase,
    pub(super) cwd: Option<&'a str>,
    pub(super) model: Option<&'a str>,
    pub(super) thread_id: Option<&'a str>,
    pub(super) turn_id: Option<&'a str>,
    pub(super) main_status: Option<&'a str>,
}

pub(super) fn build_hook_context(
    correlation_id: &str,
    metadata: &Value,
    input: HookContextInput<'_>,
) -> HookContext {
    HookContext {
        phase: input.phase,
        thread_id: input.thread_id.map(ToOwned::to_owned),
        turn_id: input.turn_id.map(ToOwned::to_owned),
        cwd: input.cwd.map(ToOwned::to_owned),
        model: input.model.map(ToOwned::to_owned),
        main_status: input.main_status.map(ToOwned::to_owned),
        correlation_id: correlation_id.to_owned(),
        ts_ms: now_millis_for_hook_context(),
        metadata: metadata.clone(),
    }
}

fn resolve_attachment_path(cwd: &str, path: &str) -> PathBuf {
    let path = PathBuf::from(path);
    if path.is_absolute() {
        path
    } else {
        PathBuf::from(cwd).join(path)
    }
}

fn hook_attachment_to_prompt_attachment(value: HookAttachment) -> PromptAttachment {
    match value {
        HookAttachment::AtPath { path, placeholder } => {
            PromptAttachment::AtPath { path, placeholder }
        }
        HookAttachment::ImageUrl { url } => PromptAttachment::ImageUrl { url },
        HookAttachment::LocalImage { path } => PromptAttachment::LocalImage { path },
        HookAttachment::Skill { name, path } => PromptAttachment::Skill { name, path },
    }
}

fn ensure_metadata_object(metadata: &mut Value) {
    if !metadata.is_object() {
        *metadata = Value::Object(Map::new());
    }
}

fn push_validation_issue(
    report: &mut HookReport,
    hook_name: &str,
    phase: HookPhase,
    message: impl Into<String>,
) {
    report.push(HookIssue {
        hook_name: hook_name.to_owned(),
        phase,
        class: HookIssueClass::Validation,
        message: message.into(),
    });
}

fn merge_metadata_delta(
    metadata: &mut Value,
    hook_name: &str,
    phase: HookPhase,
    delta: Value,
    report: &mut HookReport,
) {
    match delta {
        Value::Null => {}
        Value::Object(entries) => {
            ensure_metadata_object(metadata);
            if let Some(target) = metadata.as_object_mut() {
                for (key, value) in entries {
                    target.insert(key, value);
                }
            } else {
                push_validation_issue(
                    report,
                    hook_name,
                    phase,
                    "failed to normalize metadata object",
                );
            }
        }
        _ => push_validation_issue(
            report,
            hook_name,
            phase,
            "metadata_delta must be null or object",
        ),
    }
}

fn apply_prompt_patch(
    state: &mut PromptMutationState,
    cwd: &str,
    hook_name: &str,
    phase: HookPhase,
    patch: HookPatch,
    report: &mut HookReport,
) {
    if let Some(prompt) = patch.prompt_override {
        state.prompt = prompt;
    }
    if let Some(model) = patch.model_override {
        state.model = Some(model);
    }
    for attachment in patch.add_attachments {
        let prompt_attachment = hook_attachment_to_prompt_attachment(attachment);
        let valid = match &prompt_attachment {
            PromptAttachment::AtPath { path, .. }
            | PromptAttachment::LocalImage { path }
            | PromptAttachment::Skill { path, .. } => resolve_attachment_path(cwd, path).exists(),
            PromptAttachment::ImageUrl { .. } => true,
        };
        if valid {
            state.attachments.push(prompt_attachment);
        } else {
            push_validation_issue(
                report,
                hook_name,
                phase,
                "hook attachment path not found; mutation ignored",
            );
        }
    }
    merge_metadata_delta(
        &mut state.metadata,
        hook_name,
        phase,
        patch.metadata_delta,
        report,
    );
}

fn apply_session_patch(
    state: &mut SessionMutationState,
    hook_name: &str,
    phase: HookPhase,
    patch: HookPatch,
    report: &mut HookReport,
) {
    if patch.prompt_override.is_some() {
        push_validation_issue(
            report,
            hook_name,
            phase,
            "prompt_override is not allowed in PreSessionStart",
        );
    }
    if !patch.add_attachments.is_empty() {
        push_validation_issue(
            report,
            hook_name,
            phase,
            "add_attachments is not allowed in PreSessionStart",
        );
    }
    if let Some(model) = patch.model_override {
        state.model = Some(model);
    }
    merge_metadata_delta(
        &mut state.metadata,
        hook_name,
        phase,
        patch.metadata_delta,
        report,
    );
}

pub(super) fn apply_pre_hook_actions_to_prompt(
    state: &mut PromptMutationState,
    cwd: &str,
    phase: HookPhase,
    decisions: Vec<PreHookDecision>,
    report: &mut HookReport,
) {
    for decision in decisions {
        match decision.action {
            HookAction::Noop => {}
            HookAction::Mutate(patch) => apply_prompt_patch(
                state,
                cwd,
                decision.hook_name.as_str(),
                phase,
                patch,
                report,
            ),
        }
    }
}

pub(super) fn apply_pre_hook_actions_to_session(
    state: &mut SessionMutationState,
    phase: HookPhase,
    decisions: Vec<PreHookDecision>,
    report: &mut HookReport,
) {
    for decision in decisions {
        match decision.action {
            HookAction::Noop => {}
            HookAction::Mutate(patch) => {
                apply_session_patch(state, decision.hook_name.as_str(), phase, patch, report)
            }
        }
    }
}

pub(super) fn result_status<T, E>(result: &Result<T, E>) -> &'static str {
    if result.is_ok() {
        "ok"
    } else {
        "error"
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum LaggedTurnTerminal {
    Completed { assistant_text: Option<String> },
    Failed { message: Option<String> },
    Interrupted,
}

pub(super) fn extract_assistant_text_from_turn(turn: &ThreadTurnView) -> Option<String> {
    let mut parts = Vec::<String>::new();
    for item in &turn.items {
        if let ThreadItemPayloadView::AgentMessage(data) = &item.payload {
            if !data.text.trim().is_empty() {
                parts.push(data.text.clone());
            }
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n"))
    }
}

pub(super) fn interrupt_turn_best_effort(thread: &ThreadHandle, turn_id: &str) {
    const INTERRUPT_RPC_TIMEOUT: Duration = Duration::from_millis(500);

    let runtime = thread.runtime().clone();
    let thread_id = thread.thread_id.clone();
    let turn_id = turn_id.to_owned();
    tokio::spawn(async move {
        let mut params = Map::<String, Value>::new();
        params.insert("threadId".to_owned(), Value::String(thread_id));
        params.insert("turnId".to_owned(), Value::String(turn_id));
        let _ = runtime
            .call_raw_with_timeout(
                "turn/interrupt",
                Value::Object(params),
                INTERRUPT_RPC_TIMEOUT,
            )
            .await;
    });
}
