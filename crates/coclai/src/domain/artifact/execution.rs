use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::models::{
    apply_doc_patch, compute_revision, map_patch_conflict, validate_doc_patch, ArtifactMeta,
    ArtifactSession, ArtifactStore, ArtifactTaskKind, ArtifactTaskResult, ArtifactTaskSpec,
    DocPatch, DomainError, SaveMeta, StoreErr,
};
use super::ArtifactSessionManager;
use crate::runtime::api::{ApprovalPolicy, ReasoningEffort, SandboxPreset};
use crate::runtime::core::Runtime;
use crate::runtime::errors::RuntimeError;
use crate::runtime::events::Envelope;
use crate::runtime::rpc_contract::methods as events;
use crate::runtime::turn_output::{
    parse_thread_id, parse_turn_id, TurnStreamCollector, TurnTerminalEvent,
};
use serde_json::json;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::broadcast::Receiver as BroadcastReceiver;
use tokio::time::{timeout, Duration, Instant};

const DEFAULT_ARTIFACT_REASONING_EFFORT: ReasoningEffort = ReasoningEffort::Medium;
const MAX_TURN_EVENT_SCAN: usize = 20_000;
const TURN_OUTPUT_TIMEOUT: Duration = Duration::from_secs(120);
const INTERRUPT_RPC_TIMEOUT: Duration = Duration::from_millis(500);
const TURN_OUTPUT_FIELDS: [&str; 1] = ["output"];

// --- from orchestrator.rs ---

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
    let prompt = build_turn_prompt(&spec, &session.format, &persisted_revision, context_text);
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
    let output_json = extract_output_json(&turn_output, &["format", "title", "text"])?;
    let output: DocGenerateOutput = serde_json::from_value(output_json)
        .map_err(|err| DomainError::Parse(format!("docGenerate payload parse failed: {err}")))?;

    let new_revision = compute_revision(&output.text);
    let output_title = output.title.clone();
    let output_format = output.format.clone();
    let thread_id_for_meta = session.thread_id.clone();
    let revision_for_meta = new_revision.clone();
    persist_text_and_update_meta(
        manager,
        &spec.artifact_id,
        &output.text,
        SaveMeta {
            task_kind: ArtifactTaskKind::DocGenerate,
            thread_id: session.thread_id.clone(),
            turn_id: turn_id.clone(),
            previous_revision: Some(persisted_revision.clone()),
            next_revision: new_revision.clone(),
        },
        move |meta| {
            meta.title = output_title;
            meta.format = output_format;
            meta.revision = revision_for_meta;
            meta.runtime_thread_id = Some(thread_id_for_meta);
        },
    )
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
    let output_json = extract_output_json(&turn_output, &["format", "expectedRevision", "edits"])?;
    let patch: DocPatch = serde_json::from_value(output_json)
        .map_err(|err| DomainError::Parse(format!("docEdit patch parse failed: {err}")))?;

    let validated = validate_doc_patch(&persisted_text, &patch).map_err(map_patch_conflict)?;
    let new_text = apply_doc_patch(&persisted_text, &validated);
    let new_revision = compute_revision(&new_text);

    let patch_format = patch.format.clone();
    let thread_id_for_meta = session.thread_id.clone();
    let revision_for_meta = new_revision.clone();
    persist_text_and_update_meta(
        manager,
        &spec.artifact_id,
        &new_text,
        SaveMeta {
            task_kind: ArtifactTaskKind::DocEdit,
            thread_id: session.thread_id.clone(),
            turn_id: turn_id.clone(),
            previous_revision: Some(persisted_revision.clone()),
            next_revision: new_revision.clone(),
        },
        move |meta| {
            meta.format = patch_format;
            meta.revision = revision_for_meta;
            meta.runtime_thread_id = Some(thread_id_for_meta);
        },
    )
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

async fn persist_text_and_update_meta(
    manager: &ArtifactSessionManager,
    artifact_id: &str,
    new_text: &str,
    save_meta: SaveMeta,
    update_meta: impl FnOnce(&mut ArtifactMeta),
) -> Result<(), DomainError> {
    let artifact_id_owned = artifact_id.to_owned();
    let mut meta = manager
        .store_io({
            let artifact_id = artifact_id_owned.clone();
            move |store| store.get_meta(&artifact_id)
        })
        .await?;
    update_meta(&mut meta);
    let text_to_save = new_text.to_owned();
    manager
        .store_io(move |store| {
            store.save_text_and_meta(&artifact_id_owned, &text_to_save, save_meta, meta)
        })
        .await
}

// --- from task.rs ---

pub(crate) async fn run_turn_and_collect_output(
    runtime: &Runtime,
    thread_id: &str,
    turn_params: Value,
) -> Result<(Option<String>, Value), DomainError> {
    let mut live_rx = runtime.subscribe_live();
    let turn_start_result = runtime.call_raw(events::TURN_START, turn_params).await?;
    let turn_id = parse_turn_id(&turn_start_result);

    if let Some(output) = extract_direct_output_candidate(&turn_start_result)? {
        return Ok((turn_id, output));
    }

    let target_turn_id = turn_id.as_deref().ok_or_else(|| {
        DomainError::Parse(format!(
            "turn/start missing output and turn id in result: {}",
            turn_start_result
        ))
    })?;
    match collect_turn_output_from_live(&mut live_rx, thread_id, target_turn_id).await {
        Ok(output) => Ok((turn_id, output)),
        Err(err) => {
            interrupt_turn_best_effort(runtime, thread_id, target_turn_id).await;
            Err(err)
        }
    }
}

async fn interrupt_turn_best_effort(runtime: &Runtime, thread_id: &str, turn_id: &str) {
    let _ = runtime
        .turn_interrupt_with_timeout(thread_id, turn_id, INTERRUPT_RPC_TIMEOUT)
        .await;
}

async fn collect_turn_output_from_live(
    live_rx: &mut BroadcastReceiver<Envelope>,
    thread_id: &str,
    turn_id: &str,
) -> Result<Value, DomainError> {
    collect_turn_output_from_live_with_limits(
        live_rx,
        thread_id,
        turn_id,
        MAX_TURN_EVENT_SCAN,
        TURN_OUTPUT_TIMEOUT,
    )
    .await
}

pub(crate) async fn collect_turn_output_from_live_with_limits(
    live_rx: &mut BroadcastReceiver<Envelope>,
    thread_id: &str,
    turn_id: &str,
    max_turn_event_scan: usize,
    wait_timeout: Duration,
) -> Result<Value, DomainError> {
    let deadline = Instant::now() + wait_timeout;
    let mut turn_event_budget = max_turn_event_scan;
    let mut completed = false;
    let mut stream = TurnStreamCollector::new(thread_id, turn_id);
    let mut output_from_event: Option<Value> = None;

    while turn_event_budget > 0 {
        let now = Instant::now();
        if now >= deadline {
            return Err(DomainError::Runtime(RuntimeError::Timeout));
        }
        let remaining = deadline.saturating_duration_since(now);
        let envelope = match timeout(remaining, live_rx.recv()).await {
            Ok(Ok(envelope)) => envelope,
            Ok(Err(RecvError::Lagged(_))) => continue,
            Ok(Err(RecvError::Closed)) => {
                return Err(DomainError::Runtime(RuntimeError::Internal(format!(
                    "live stream closed while waiting turn output: {}",
                    RecvError::Closed
                ))));
            }
            Err(_) => return Err(DomainError::Runtime(RuntimeError::Timeout)),
        };

        if !stream.is_target_envelope(&envelope) {
            continue;
        }
        turn_event_budget = turn_event_budget.saturating_sub(1);

        if output_from_event.is_none() {
            let params = envelope.json.get("params").cloned().unwrap_or(Value::Null);
            output_from_event = extract_output_candidate_from_params(&params)?;
        }

        match stream.push_envelope(&envelope) {
            Some(TurnTerminalEvent::Completed) => {
                completed = true;
                break;
            }
            Some(TurnTerminalEvent::Failed) => {
                return Err(DomainError::Validation(format!(
                    "turn failed while collecting output: turn_id={turn_id}"
                )));
            }
            Some(TurnTerminalEvent::Interrupted) | Some(TurnTerminalEvent::Cancelled) => {
                return Err(DomainError::Validation(format!(
                    "turn interrupted while collecting output: turn_id={turn_id}"
                )));
            }
            None => {}
        }
    }

    if !completed && turn_event_budget == 0 {
        return Err(DomainError::Parse(format!(
            "turn output scan exceeded event budget: turn_id={turn_id}"
        )));
    }

    if let Some(output) = output_from_event {
        return Ok(output);
    }

    parse_json_output_text(stream.assistant_text())
}

fn extract_direct_output_candidate(
    turn_start_result: &Value,
) -> Result<Option<Value>, DomainError> {
    for key in TURN_OUTPUT_FIELDS {
        if let Some(candidate) = turn_start_result.get(key) {
            return normalize_output_candidate(candidate).map(Some);
        }
    }

    if turn_start_result.is_string() {
        return normalize_output_candidate(turn_start_result).map(Some);
    }

    let Some(obj) = turn_start_result.as_object() else {
        return Ok(None);
    };
    if obj.contains_key("turn") || obj.contains_key("thread") {
        return Ok(None);
    }
    Ok(Some(turn_start_result.clone()))
}

fn extract_output_candidate_from_params(params: &Value) -> Result<Option<Value>, DomainError> {
    for key in TURN_OUTPUT_FIELDS {
        if let Some(candidate) = params.get(key) {
            return normalize_output_candidate(candidate).map(Some);
        }
    }
    if let Some(item) = params.get("item") {
        for key in TURN_OUTPUT_FIELDS {
            if let Some(candidate) = item.get(key) {
                return normalize_output_candidate(candidate).map(Some);
            }
        }
    }
    Ok(None)
}

fn parse_json_output_text(text: &str) -> Result<Value, DomainError> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(DomainError::Parse(
            "turn completed without structured output".to_owned(),
        ));
    }
    if let Ok(parsed) = serde_json::from_str::<Value>(trimmed) {
        return Ok(parsed);
    }
    if let Some(fenced) = extract_fenced_json(trimmed) {
        if let Ok(parsed) = serde_json::from_str::<Value>(&fenced) {
            return Ok(parsed);
        }
    }
    Err(DomainError::Parse(format!(
        "turn output is not valid JSON: {}",
        trimmed
    )))
}

fn extract_fenced_json(text: &str) -> Option<String> {
    let mut lines = text.lines();
    let first = lines.next()?;
    if !first.starts_with("```") {
        return None;
    }

    let mut out = String::new();
    for line in lines {
        if line.starts_with("```") {
            break;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(line);
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

pub(crate) fn load_or_default_meta(
    store: &dyn ArtifactStore,
    artifact_id: &str,
) -> Result<ArtifactMeta, StoreErr> {
    let text = match store.load_text(artifact_id) {
        Ok(value) => value,
        Err(StoreErr::NotFound(_)) => String::new(),
        Err(err) => return Err(err),
    };
    let actual_revision = compute_revision(&text);

    match store.get_meta(artifact_id) {
        Ok(mut meta) => {
            if meta.revision != actual_revision {
                meta.revision = actual_revision;
            }
            Ok(meta)
        }
        Err(StoreErr::NotFound(_)) => Ok(ArtifactMeta {
            title: artifact_id.to_owned(),
            format: "markdown".to_owned(),
            revision: actual_revision,
            runtime_thread_id: None,
        }),
        Err(err) => Err(err),
    }
}

pub(crate) async fn start_thread(runtime: &Runtime) -> Result<String, DomainError> {
    let result = runtime.call_raw(events::THREAD_START, json!({})).await?;
    parse_thread_id(&result).ok_or_else(|| {
        DomainError::Parse(format!(
            "thread/start missing thread id in result: {}",
            result
        ))
    })
}

pub(crate) async fn resume_thread(
    runtime: &Runtime,
    thread_id: &str,
) -> Result<String, DomainError> {
    let result = runtime
        .call_raw(events::THREAD_RESUME, json!({ "threadId": thread_id }))
        .await?;
    let resumed = parse_thread_id(&result).ok_or_else(|| {
        DomainError::Parse(format!(
            "thread/resume missing thread id in result: {}",
            result
        ))
    })?;
    if resumed != thread_id {
        return Err(DomainError::Parse(format!(
            "thread/resume returned mismatched thread id: requested={thread_id} actual={resumed}"
        )));
    }
    Ok(resumed)
}

/// Build deterministic domain prompt text.
/// Allocation: one String buffer. Complexity: O(L + c + e), L=text size, c=constraints, e=examples.
pub fn build_turn_prompt(
    spec: &ArtifactTaskSpec,
    format: &str,
    revision: &str,
    current_text: &str,
) -> String {
    let mut prompt = String::with_capacity(current_text.len().saturating_add(512));
    prompt.push_str("ROLE:\n");
    prompt.push_str(
        "You are a documentation/rules engine. Do NOT use tools. Output JSON matching the schema only.\n\n",
    );

    prompt.push_str("GOAL:\n");
    prompt.push_str(spec.user_goal.trim());
    prompt.push_str("\n\n");

    prompt.push_str("CONSTRAINTS:\n");
    if spec.constraints.is_empty() {
        prompt.push_str("- none\n");
    } else {
        for c in &spec.constraints {
            prompt.push_str("- ");
            prompt.push_str(c);
            prompt.push('\n');
        }
    }
    prompt.push('\n');

    prompt.push_str("CONTEXT:\n");
    prompt.push_str("FORMAT: ");
    prompt.push_str(format);
    prompt.push('\n');
    prompt.push_str("REVISION: ");
    prompt.push_str(revision);
    prompt.push('\n');

    if !spec.examples.is_empty() {
        prompt.push_str("EXAMPLES:\n");
        for ex in &spec.examples {
            prompt.push_str("- ");
            prompt.push_str(ex);
            prompt.push('\n');
        }
    }

    prompt.push_str("CURRENT_TEXT_BEGIN\n");
    prompt.push_str(current_text);
    if !current_text.ends_with('\n') {
        prompt.push('\n');
    }
    prompt.push_str("CURRENT_TEXT_END\n");
    prompt
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ArtifactTurnStartPayload<'a> {
    thread_id: &'a str,
    input: [ArtifactInputText<'a>; 1],
    approval_policy: &'static str,
    sandbox_policy: SandboxPolicyPreset,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<&'a str>,
    effort: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    summary: Option<&'a str>,
    output_schema: &'a Value,
}

#[derive(Serialize)]
struct ArtifactInputText<'a> {
    #[serde(rename = "type")]
    item_type: &'static str,
    text: &'a str,
}

#[derive(Serialize)]
struct SandboxPolicyPreset {
    #[serde(rename = "type")]
    preset_type: &'static str,
}

/// Build turn/start params with fixed safe policy.
/// Side effects: none. Allocation: Serialized directly to JSON Value.
pub fn build_turn_start_params(thread_id: &str, prompt: &str, spec: &ArtifactTaskSpec) -> Value {
    let effort = spec.effort.unwrap_or(DEFAULT_ARTIFACT_REASONING_EFFORT);

    let payload = ArtifactTurnStartPayload {
        thread_id,
        input: [ArtifactInputText {
            item_type: "text",
            text: prompt,
        }],
        approval_policy: ApprovalPolicy::Never.as_wire(),
        sandbox_policy: SandboxPolicyPreset {
            preset_type: SandboxPreset::ReadOnly.as_type_wire(),
        },
        model: spec.model.as_deref(),
        effort: effort.as_wire(),
        summary: spec.summary.as_deref(),
        output_schema: &spec.output_schema,
    };

    serde_json::to_value(&payload).unwrap_or(Value::Null)
}

pub(crate) fn extract_output_json(
    turn_result: &Value,
    required_keys: &[&str],
) -> Result<Value, DomainError> {
    if has_required_keys(turn_result, required_keys) {
        return Ok(turn_result.clone());
    }

    for key in TURN_OUTPUT_FIELDS {
        if let Some(candidate) = turn_result.get(key) {
            let parsed = normalize_output_candidate(candidate)?;
            if has_required_keys(&parsed, required_keys) {
                return Ok(parsed);
            }
        }
    }

    Err(DomainError::Parse(format!(
        "turn output missing required keys {:?}: {}",
        required_keys, turn_result
    )))
}

fn normalize_output_candidate(candidate: &Value) -> Result<Value, DomainError> {
    match candidate {
        Value::String(text) => serde_json::from_str::<Value>(text)
            .map_err(|err| DomainError::Parse(format!("output JSON parse failed: {err}"))),
        Value::Object(_) | Value::Array(_) => Ok(candidate.clone()),
        _ => Err(DomainError::Parse(format!(
            "output candidate must be object/array/string JSON: {}",
            candidate
        ))),
    }
}

fn has_required_keys(value: &Value, required_keys: &[&str]) -> bool {
    let Some(obj) = value.as_object() else {
        return false;
    };
    required_keys.iter().all(|key| obj.contains_key(*key))
}
