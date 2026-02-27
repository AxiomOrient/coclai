use coclai_runtime::api::ReasoningEffort;
use coclai_runtime::errors::RuntimeError;
use coclai_runtime::events::Envelope;
use coclai_runtime::runtime::Runtime;
use coclai_runtime::turn_output::{parse_thread_id, parse_turn_id, AssistantTextCollector};
use serde_json::{json, Map, Value};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::broadcast::Receiver as BroadcastReceiver;
use tokio::time::{timeout, Duration, Instant};

use super::patch::compute_revision;
use super::{ArtifactMeta, ArtifactStore, ArtifactTaskSpec, DomainError, StoreErr};

const DEFAULT_ARTIFACT_REASONING_EFFORT: ReasoningEffort = ReasoningEffort::Medium;
const MAX_TURN_EVENT_SCAN: usize = 20_000;
const TURN_OUTPUT_TIMEOUT: Duration = Duration::from_secs(120);
const INTERRUPT_RPC_TIMEOUT: Duration = Duration::from_millis(500);
const TURN_OUTPUT_FIELDS: [&str; 1] = ["output"];

pub(crate) async fn run_turn_and_collect_output(
    runtime: &Runtime,
    thread_id: &str,
    turn_params: Value,
) -> Result<(Option<String>, Value), DomainError> {
    let mut live_rx = runtime.subscribe_live();
    let turn_start_result = runtime.call_raw("turn/start", turn_params).await?;
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
    let mut collector = AssistantTextCollector::new();
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

        if envelope.thread_id.as_deref() != Some(thread_id) {
            continue;
        }
        if envelope.turn_id.as_deref() != Some(turn_id) {
            continue;
        }
        turn_event_budget = turn_event_budget.saturating_sub(1);

        collector.push_envelope(&envelope);

        if output_from_event.is_none() {
            let params = envelope.json.get("params").cloned().unwrap_or(Value::Null);
            output_from_event = extract_output_candidate_from_params(&params)?;
        }

        match envelope.method.as_deref() {
            Some("turn/completed") => {
                completed = true;
                break;
            }
            Some("turn/failed") => {
                return Err(DomainError::Validation(format!(
                    "turn failed while collecting output: turn_id={turn_id}"
                )));
            }
            Some("turn/interrupted") => {
                return Err(DomainError::Validation(format!(
                    "turn interrupted while collecting output: turn_id={turn_id}"
                )));
            }
            _ => {}
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

    parse_json_output_text(collector.text())
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
    if !text.starts_with("```") {
        return None;
    }

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
    let result = runtime.call_raw("thread/start", json!({})).await?;
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
        .call_raw("thread/resume", json!({ "threadId": thread_id }))
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

/// Build turn/start params with fixed safe policy.
/// Side effects: none. Allocation: JSON map/object for request.
pub fn build_turn_start_params(thread_id: &str, prompt: &str, spec: &ArtifactTaskSpec) -> Value {
    let mut params = Map::<String, Value>::new();
    params.insert("threadId".to_owned(), Value::String(thread_id.to_owned()));
    params.insert(
        "input".to_owned(),
        json!([{
            "type": "text",
            "text": prompt
        }]),
    );

    // Domain default mode is fixed: never + readOnly.
    params.insert(
        "approvalPolicy".to_owned(),
        Value::String("never".to_owned()),
    );
    params.insert("sandboxPolicy".to_owned(), json!({ "type": "readOnly" }));

    if let Some(model) = spec.model.as_ref() {
        params.insert("model".to_owned(), Value::String(model.clone()));
    }
    let effort = spec.effort.unwrap_or(DEFAULT_ARTIFACT_REASONING_EFFORT);
    params.insert(
        "effort".to_owned(),
        Value::String(effort.as_wire().to_owned()),
    );
    if let Some(summary) = spec.summary.as_ref() {
        params.insert("summary".to_owned(), Value::String(summary.clone()));
    }
    params.insert("outputSchema".to_owned(), spec.output_schema.clone());
    Value::Object(params)
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
