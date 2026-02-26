use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use coclai_runtime::{
    Envelope, Runtime, RuntimeConfig, SchemaGuardConfig, ServerRequest, ServerRequestConfig,
    StdioProcessSpec, TimeoutAction,
};
use pretty_assertions::assert_eq;
use serde_json::{json, Map, Value};
use tokio::sync::{broadcast, mpsc};
use tokio::time::timeout;

const TURN_TIMEOUT: Duration = Duration::from_secs(90);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(90);
const RPC_TIMEOUT: Duration = Duration::from_secs(45);
const TEST_TIMEOUT: Duration = Duration::from_secs(180);

#[derive(Debug)]
struct TempDir {
    root: PathBuf,
}

impl TempDir {
    fn new(prefix: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        let root = std::env::temp_dir().join(format!("{prefix}_{nanos}"));
        fs::create_dir_all(&root).expect("create temp dir");
        Self { root }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn env_flag(name: &str) -> bool {
    matches!(
        std::env::var(name).ok().as_deref(),
        Some("1") | Some("true") | Some("TRUE") | Some("yes") | Some("YES")
    )
}

fn contract_enabled() -> bool {
    env_flag("APP_SERVER_CONTRACT")
}

fn update_golden_enabled() -> bool {
    env_flag("APP_SERVER_CONTRACT_UPDATE_GOLDEN")
}

fn model_dependent_contract_enabled() -> bool {
    env_flag("APP_SERVER_CONTRACT_MODEL_DEPENDENT")
}

fn contract_model() -> Option<String> {
    std::env::var("APP_SERVER_CONTRACT_MODEL").ok()
}

fn model_dependent_strict() -> bool {
    env_flag("APP_SERVER_CONTRACT_MODEL_STRICT")
}

fn contract_model_effort() -> String {
    let effort = std::env::var("APP_SERVER_CONTRACT_MODEL_EFFORT")
        .unwrap_or_else(|_| "medium".to_owned())
        .trim()
        .to_lowercase();
    match effort.as_str() {
        "low" | "medium" | "high" | "xhigh" => effort,
        other => panic!(
            "invalid APP_SERVER_CONTRACT_MODEL_EFFORT={other}; expected low|medium|high|xhigh"
        ),
    }
}

fn cli_bin() -> String {
    std::env::var("APP_SERVER_BIN").unwrap_or_else(|_| "codex".to_owned())
}

fn maybe_skip_contract() -> bool {
    if contract_enabled() {
        return false;
    }

    eprintln!("skipping real CLI contract test; set APP_SERVER_CONTRACT=1 to enable this test");
    true
}

fn maybe_skip_model_dependent(test_name: &str) -> bool {
    if !model_dependent_contract_enabled() {
        eprintln!(
            "skipping model-dependent contract test `{test_name}`; set APP_SERVER_CONTRACT_MODEL_DEPENDENT=1 to enable"
        );
        return true;
    }

    if contract_model().is_none() {
        eprintln!(
            "skipping model-dependent contract test `{test_name}`; set APP_SERVER_CONTRACT_MODEL=<model> for deterministic coverage"
        );
        return true;
    }

    false
}

fn workspace_schema_guard() -> SchemaGuardConfig {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let active = manifest_dir.join("../../SCHEMAS/app-server/active");
    SchemaGuardConfig {
        active_schema_dir: active,
    }
}

fn cli_process_spec() -> StdioProcessSpec {
    let mut spec = StdioProcessSpec::new(cli_bin());
    spec.args = vec!["app-server".to_owned()];
    spec
}

fn runtime_config() -> RuntimeConfig {
    let mut cfg = RuntimeConfig::new(cli_process_spec(), workspace_schema_guard());
    cfg.server_requests = ServerRequestConfig {
        default_timeout_ms: 60_000,
        on_timeout: TimeoutAction::Decline,
        auto_decline_unknown: false,
    };
    cfg
}

fn parse_thread_id(result: &Value) -> Option<String> {
    result
        .pointer("/thread/id")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| {
            result
                .get("threadId")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .or_else(|| {
            result
                .get("id")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
}

fn parse_turn_id(result: &Value) -> Option<String> {
    result
        .pointer("/turn/id")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| {
            result
                .get("turnId")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
        .or_else(|| {
            result
                .get("id")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        })
}

#[derive(Debug)]
struct ThreadStartContext {
    expected_approval_policy: String,
    observed_approval_policy: Option<String>,
    observed_model: Option<String>,
    observed_model_provider: Option<String>,
    observed_sandbox: Option<String>,
}

fn compact_json_value(value: &Value) -> String {
    match value {
        Value::String(text) => text.to_owned(),
        _ => serde_json::to_string(value).unwrap_or_else(|_| "<invalid-json>".to_owned()),
    }
}

fn parse_thread_start_context(
    thread_result: &Value,
    expected_approval_policy: &str,
) -> ThreadStartContext {
    ThreadStartContext {
        expected_approval_policy: expected_approval_policy.to_owned(),
        observed_approval_policy: thread_result
            .get("approvalPolicy")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        observed_model: thread_result
            .get("model")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        observed_model_provider: thread_result
            .get("modelProvider")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        observed_sandbox: thread_result.get("sandbox").map(compact_json_value),
    }
}

fn assert_strict_thread_start_preconditions(context: &ThreadStartContext) {
    if !model_dependent_strict() {
        return;
    }
    if context.observed_model.is_none() {
        panic!("strict lane precondition failed: thread model missing; thread_start={context:?}");
    }
    if context.observed_model_provider.is_none() {
        panic!(
            "strict lane precondition failed: thread modelProvider missing; thread_start={context:?}"
        );
    }
    if context.observed_sandbox.is_none() {
        panic!("strict lane precondition failed: thread sandbox missing; thread_start={context:?}");
    }
    if context.observed_approval_policy.as_deref()
        != Some(context.expected_approval_policy.as_str())
    {
        panic!(
            "strict lane precondition failed: thread approvalPolicy mismatch expected={} observed={:?}; thread_start={context:?}",
            context.expected_approval_policy,
            context.observed_approval_policy
        );
    }
}

fn thread_start_params(approval_policy: &str, sandbox: &str, cwd: Option<&Path>) -> Value {
    let mut params = Map::<String, Value>::new();
    params.insert(
        "approvalPolicy".to_owned(),
        Value::String(approval_policy.to_owned()),
    );
    params.insert("sandbox".to_owned(), Value::String(sandbox.to_owned()));
    if let Some(cwd) = cwd {
        params.insert(
            "cwd".to_owned(),
            Value::String(cwd.to_string_lossy().to_string()),
        );
    }
    if let Some(model) = contract_model() {
        params.insert("model".to_owned(), Value::String(model));
    }
    Value::Object(params)
}

fn turn_start_params(
    thread_id: &str,
    prompt: &str,
    approval_policy: &str,
    sandbox_policy: Value,
    cwd: Option<&Path>,
    output_schema: Value,
) -> Value {
    let mut params = Map::<String, Value>::new();
    params.insert("threadId".to_owned(), Value::String(thread_id.to_owned()));
    params.insert(
        "approvalPolicy".to_owned(),
        Value::String(approval_policy.to_owned()),
    );
    params.insert("sandboxPolicy".to_owned(), sandbox_policy);
    params.insert("outputSchema".to_owned(), output_schema);
    params.insert(
        "input".to_owned(),
        json!([{
            "type":"text",
            "text": prompt
        }]),
    );
    if let Some(cwd) = cwd {
        params.insert(
            "cwd".to_owned(),
            Value::String(cwd.to_string_lossy().to_string()),
        );
    }
    if let Some(model) = contract_model() {
        params.insert("model".to_owned(), Value::String(model));
    }
    params.insert("effort".to_owned(), Value::String(contract_model_effort()));
    Value::Object(params)
}

async fn wait_turn_terminal_methods(
    live_rx: &mut broadcast::Receiver<Envelope>,
    thread_id: &str,
    expected_turn_id: Option<&str>,
) -> Vec<String> {
    let mut methods = Vec::new();

    loop {
        let envelope = timeout(TURN_TIMEOUT, live_rx.recv())
            .await
            .expect("live wait timeout")
            .expect("live channel closed");

        if envelope.thread_id.as_deref() != Some(thread_id) {
            continue;
        }

        if let Some(method) = envelope.method.as_deref() {
            methods.push(method.to_owned());
            if matches!(
                method,
                "turn/completed" | "turn/failed" | "turn/interrupted"
            ) {
                if let Some(target_turn_id) = expected_turn_id {
                    if envelope.turn_id.as_deref() != Some(target_turn_id) {
                        continue;
                    }
                }
                break;
            }
        }
    }

    methods
}

#[derive(Debug)]
enum ApprovalObservation {
    Requested(ServerRequest),
    TurnTerminated {
        terminal_method: String,
        observed_methods: Vec<String>,
        error_messages: Vec<String>,
    },
}

#[allow(dead_code)]
#[derive(Debug)]
struct PromptAttemptTerminal {
    prompt: String,
    terminal_method: String,
    observed_methods: Vec<String>,
    error_messages: Vec<String>,
}

/// Wait until either approval request arrives or turn reaches terminal state.
/// Side effects: consumes request/event streams. Allocation: O(k), k = observed methods count.
/// Complexity: O(k) stream steps.
async fn wait_approval_or_turn_terminal(
    request_rx: &mut mpsc::Receiver<ServerRequest>,
    live_rx: &mut broadcast::Receiver<Envelope>,
    thread_id: &str,
    expected_turn_id: Option<&str>,
) -> Result<ApprovalObservation, String> {
    let mut observed_methods = Vec::<String>::new();
    let mut error_messages = Vec::<String>::new();

    let outcome = timeout(REQUEST_TIMEOUT, async {
        loop {
            tokio::select! {
                request = request_rx.recv() => {
                    match request {
                        Some(request) => return Ok(ApprovalObservation::Requested(request)),
                        None => {
                            return Err("approval request channel closed".to_owned());
                        }
                    }
                }
                envelope = live_rx.recv() => {
                    let envelope = match envelope {
                        Ok(envelope) => envelope,
                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(broadcast::error::RecvError::Closed) => {
                            return Err("live channel closed".to_owned());
                        }
                    };

                    if envelope.thread_id.as_deref() != Some(thread_id) {
                        continue;
                    }
                    let Some(method) = envelope.method.as_deref() else {
                        continue;
                    };
                    observed_methods.push(method.to_owned());
                    if method == "error" {
                        if let Some(message) = extract_error_message(&envelope.json) {
                            error_messages.push(message);
                        }
                    }

                    if matches!(method, "turn/completed" | "turn/failed" | "turn/interrupted") {
                        if let Some(target_turn_id) = expected_turn_id {
                            if envelope.turn_id.as_deref() != Some(target_turn_id) {
                                continue;
                            }
                        }
                        return Ok(ApprovalObservation::TurnTerminated {
                            terminal_method: method.to_owned(),
                            observed_methods,
                            error_messages,
                        });
                    }
                }
            }
        }
    })
    .await;

    match outcome {
        Ok(result) => result,
        Err(_) => Err("approval/terminal wait timeout".to_owned()),
    }
}

/// Start turns with prompt candidates until approval arrives, or all candidates terminate.
/// Side effects: sends turn/start RPC calls and consumes request/event streams.
/// Complexity: O(P * E), P = prompt count, E = events until each terminal/request.
#[allow(clippy::too_many_arguments)]
async fn wait_approval_with_prompt_candidates(
    runtime: &Runtime,
    request_rx: &mut mpsc::Receiver<ServerRequest>,
    live_rx: &mut broadcast::Receiver<Envelope>,
    thread_id: &str,
    prompts: &[&str],
    approval_policy: &str,
    sandbox_policy: &Value,
    cwd: Option<&Path>,
    output_schema: &Value,
) -> Result<(ServerRequest, Option<String>), Vec<PromptAttemptTerminal>> {
    let mut failures = Vec::<PromptAttemptTerminal>::new();

    for prompt in prompts {
        let turn_result = call_raw_with_timeout(
            runtime,
            "turn/start",
            turn_start_params(
                thread_id,
                prompt,
                approval_policy,
                sandbox_policy.clone(),
                cwd,
                output_schema.clone(),
            ),
            RPC_TIMEOUT,
        )
        .await;
        let turn_id = parse_turn_id(&turn_result);

        let observation =
            wait_approval_or_turn_terminal(request_rx, live_rx, thread_id, turn_id.as_deref())
                .await;

        match observation {
            Ok(ApprovalObservation::Requested(req)) => return Ok((req, turn_id)),
            Ok(ApprovalObservation::TurnTerminated {
                terminal_method,
                observed_methods,
                error_messages,
            }) => failures.push(PromptAttemptTerminal {
                prompt: (*prompt).to_owned(),
                terminal_method,
                observed_methods: dedup_methods(observed_methods),
                error_messages: dedup_methods(error_messages),
            }),
            Err(err) => failures.push(PromptAttemptTerminal {
                prompt: (*prompt).to_owned(),
                terminal_method: "observation_error".to_owned(),
                observed_methods: Vec::new(),
                error_messages: vec![err],
            }),
        }
    }

    Err(failures)
}

async fn call_raw_with_timeout(
    runtime: &Runtime,
    method: &str,
    params: Value,
    timeout_dur: Duration,
) -> Value {
    timeout(timeout_dur, runtime.call_raw(method, params))
        .await
        .unwrap_or_else(|_| panic!("rpc timeout: {method}"))
        .unwrap_or_else(|err| panic!("rpc failed for {method}: {err}"))
}

fn dedup_methods(methods: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::<String>::new();
    let mut deduped = Vec::new();
    for method in methods {
        if seen.insert(method.clone()) {
            deduped.push(method);
        }
    }
    deduped
}

fn prompt_failures_upstream_inconclusive(failures: &[PromptAttemptTerminal]) -> bool {
    if failures.is_empty() {
        return false;
    }

    failures.iter().all(|failure| {
        failure.error_messages.iter().any(|msg| {
            msg.contains("stream disconnected before completion")
                || msg.contains("approval/terminal wait timeout")
                || msg.contains("live channel closed")
        })
    })
}

fn prompt_failures_model_incompatible(
    failures: &[PromptAttemptTerminal],
    expected_request_method: &str,
) -> Option<String> {
    for failure in failures {
        for msg in &failure.error_messages {
            let normalized = msg.to_ascii_lowercase();
            let unsupported_model = normalized
                .contains("model is not supported when using codex with a chatgpt account");
            let unsupported_effort = (normalized.contains("unsupported value")
                || normalized.contains("not supported"))
                && normalized.contains("reasoning.effort");
            if unsupported_model || unsupported_effort {
                return Some(msg.clone());
            }
        }
    }

    // Some model/runtime pairs complete turns but never emit the expected request event.
    if failures.is_empty() {
        return None;
    }
    let all_completed = failures
        .iter()
        .all(|failure| failure.terminal_method == "turn/completed");
    if !all_completed {
        return None;
    }

    let mut observed_methods = HashSet::<String>::new();
    for failure in failures {
        for method in &failure.observed_methods {
            observed_methods.insert(method.clone());
        }
    }
    if observed_methods.contains(expected_request_method) {
        return None;
    }

    if expected_request_method == "item/fileChange/requestApproval"
        && observed_methods.contains("item/fileChange/outputDelta")
    {
        return Some(
            "file-change output appeared (`item/fileChange/outputDelta`) but approval request event (`item/fileChange/requestApproval`) never appeared".to_owned(),
        );
    }
    if expected_request_method == "item/commandExecution/requestApproval"
        && !observed_methods
            .iter()
            .any(|method| method.starts_with("item/commandExecution/"))
    {
        return Some(
            "turns completed without any commandExecution events; command tool routing appears unavailable for this model/runtime".to_owned(),
        );
    }
    if expected_request_method == "item/tool/requestUserInput"
        && !observed_methods.contains("item/tool/requestUserInput")
    {
        return Some(
            "turns completed without `item/tool/requestUserInput`; requestUserInput tool routing appears unavailable for this model/runtime".to_owned(),
        );
    }

    Some(format!(
        "expected `{expected_request_method}` but turns completed without emitting it"
    ))
}

fn extract_error_message(payload: &Value) -> Option<String> {
    let params = payload.get("params");
    let roots = [
        params.and_then(|v| v.get("error")),
        payload.get("error"),
        params,
        Some(payload),
    ];

    for root in roots.into_iter().flatten() {
        let message = root
            .get("message")
            .and_then(Value::as_str)
            .or_else(|| root.get("detail").and_then(Value::as_str))
            .or_else(|| root.get("reason").and_then(Value::as_str))
            .or_else(|| root.get("text").and_then(Value::as_str))
            .or_else(|| {
                root.get("error")
                    .and_then(|v| v.get("message"))
                    .and_then(Value::as_str)
            });
        if let Some(message) = message {
            return Some(message.to_owned());
        }
    }

    None
}

fn contract_turn_milestones(methods: Vec<String>) -> Vec<String> {
    methods
        .into_iter()
        .filter(|method| {
            matches!(
                method.as_str(),
                "thread/started" | "turn/started" | "turn/completed"
            )
        })
        .collect()
}

fn golden_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../SCHEMAS/golden/events")
        .join(name)
}

fn assert_or_update_golden(path: &Path, actual: &Value) {
    if update_golden_enabled() {
        let body = serde_json::to_string_pretty(actual).expect("serialize golden");
        fs::write(path, format!("{body}\n")).expect("write golden");
        return;
    }

    let raw = fs::read_to_string(path).expect("read golden");
    let expected: Value = serde_json::from_str(&raw).expect("parse golden");
    assert_eq!(
        actual, &expected,
        "golden mismatch at {:?}; set APP_SERVER_CONTRACT_UPDATE_GOLDEN=1 to refresh",
        path
    );
}

fn sorted_param_keys(params: &Value) -> Vec<String> {
    let mut keys = params
        .as_object()
        .map(|obj| obj.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    keys.sort();
    keys
}

async fn with_test_timeout(name: &str, f: impl std::future::Future<Output = ()>) {
    timeout(TEST_TIMEOUT, f)
        .await
        .unwrap_or_else(|_| panic!("contract test timed out: {name}"));
}

#[tokio::test(flavor = "current_thread")]
async fn contract_simple_turn_stream_matches_golden() {
    if maybe_skip_contract() {
        return;
    }

    with_test_timeout("contract_simple_turn_stream_matches_golden", async {
        let runtime = Runtime::spawn_local(runtime_config())
            .await
            .expect("spawn runtime");
        let mut live_rx = runtime.subscribe_live();

        let thread_result = call_raw_with_timeout(
            &runtime,
            "thread/start",
            thread_start_params("never", "read-only", None),
            RPC_TIMEOUT,
        )
        .await;
        let thread_id = parse_thread_id(&thread_result)
            .unwrap_or_else(|| panic!("thread/start missing thread id: {thread_result}"));

        let turn_result = call_raw_with_timeout(
            &runtime,
            "turn/start",
            turn_start_params(
                &thread_id,
                "Reply with JSON only: {\"status\":\"ok\"}. Do not use tools.",
                "never",
                json!({ "type": "readOnly" }),
                None,
                json!({
                    "type":"object",
                    "required":["status"],
                    "properties":{"status":{"type":"string"}},
                    "additionalProperties": false
                }),
            ),
            RPC_TIMEOUT,
        )
        .await;
        let turn_id = parse_turn_id(&turn_result);

        let methods =
            wait_turn_terminal_methods(&mut live_rx, &thread_id, turn_id.as_deref()).await;
        let deduped = dedup_methods(methods);
        let milestones = contract_turn_milestones(deduped);

        assert!(
            milestones.iter().any(|m| m == "thread/started"),
            "missing thread/started in stream"
        );
        assert!(
            milestones.iter().any(|m| m == "turn/started"),
            "missing turn/started in stream"
        );
        assert!(
            milestones.iter().any(|m| m == "turn/completed"),
            "missing turn/completed in stream"
        );

        assert_or_update_golden(
            &golden_path("simple_turn_stream.json"),
            &Value::Array(milestones.into_iter().map(Value::String).collect()),
        );

        runtime.shutdown().await.expect("shutdown");
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn contract_command_approval_matches_golden() {
    if maybe_skip_contract()
        || maybe_skip_model_dependent("contract_command_approval_matches_golden")
    {
        return;
    }

    with_test_timeout("contract_command_approval_matches_golden", async {
        let cwd = TempDir::new("contract_cmd");
        let runtime = Runtime::spawn_local(runtime_config())
            .await
            .expect("spawn runtime");
        let mut live_rx = runtime.subscribe_live();
        let mut request_rx = runtime
            .take_server_request_rx()
            .await
            .expect("take request rx");

        let thread_result = call_raw_with_timeout(
            &runtime,
            "thread/start",
            thread_start_params("on-request", "workspace-write", Some(&cwd.root)),
            RPC_TIMEOUT,
        )
        .await;
        let thread_ctx = parse_thread_start_context(&thread_result, "on-request");
        assert_strict_thread_start_preconditions(&thread_ctx);
        let thread_id = parse_thread_id(&thread_result)
            .unwrap_or_else(|| panic!("thread/start missing thread id: {thread_result}"));

        let command_prompts = [
            "First action: call the command execution tool exactly once with `pwd` in the workspace. Do not answer before the tool call.",
            "You must request command approval for running `pwd` before any final response.",
            "Task requirement: use a command tool call (`pwd`) now; skipping the tool call is invalid.",
            "Emit a command execution request now so that `item/commandExecution/requestApproval` is produced for `pwd`.",
            "Do not provide final JSON yet. First produce a command tool call for `pwd`.",
        ];
        let command_sandbox_policy = json!({
            "type":"workspaceWrite",
            "writableRoots":[cwd.root.to_string_lossy().to_string()],
            "networkAccess": false
        });
        let command_output_schema = json!({
            "type":"object",
            "required":["status"],
            "properties":{"status":{"type":"string"}},
            "additionalProperties": false
        });

        let (req, turn_id) = match wait_approval_with_prompt_candidates(
            &runtime,
            &mut request_rx,
            &mut live_rx,
            &thread_id,
            &command_prompts,
            "on-request",
            &command_sandbox_policy,
            Some(&cwd.root),
            &command_output_schema,
        )
        .await
        {
            Ok(found) => found,
            Err(failures) => {
                if model_dependent_strict() {
                    if prompt_failures_upstream_inconclusive(&failures) {
                        panic!(
                            "strict lane inconclusive: upstream stream disconnected or stalled across {} prompt(s); thread_start={thread_ctx:?}; failures={failures:?}",
                            failures.len()
                        );
                    }
                    if let Some(reason) = prompt_failures_model_incompatible(
                        &failures,
                        "item/commandExecution/requestApproval",
                    ) {
                        panic!(
                            "strict lane incompatible model/runtime configuration: reason={reason}; thread_start={thread_ctx:?}; failures={failures:?}"
                        );
                    }
                    panic!(
                        "expected command approval but no approval request across {} prompt(s); thread_start={thread_ctx:?}; failures={failures:?}",
                        failures.len()
                    );
                }
                eprintln!(
                    "skipping model-dependent command approval assertion: no approval request across {} prompt(s); thread_start={thread_ctx:?}; failures={failures:?}. Set APP_SERVER_CONTRACT_MODEL_STRICT=1 to fail in this case.",
                    failures.len(),
                );
                runtime.shutdown().await.expect("shutdown");
                return;
            }
        };
        assert_eq!(req.method, "item/commandExecution/requestApproval");

        runtime
            .respond_approval_ok(&req.approval_id, json!({ "decision": "decline" }))
            .await
            .expect("respond approval");

        let _ = wait_turn_terminal_methods(&mut live_rx, &thread_id, turn_id.as_deref()).await;

        let actual = json!({
            "method": req.method,
            "paramKeys": sorted_param_keys(&req.params),
        });
        assert_or_update_golden(&golden_path("approval_command_request.json"), &actual);

        runtime.shutdown().await.expect("shutdown");
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn contract_file_approval_matches_golden() {
    if maybe_skip_contract() || maybe_skip_model_dependent("contract_file_approval_matches_golden")
    {
        return;
    }

    with_test_timeout("contract_file_approval_matches_golden", async {
        let cwd = TempDir::new("contract_file");
        let runtime = Runtime::spawn_local(runtime_config())
            .await
            .expect("spawn runtime");
        let mut live_rx = runtime.subscribe_live();
        let mut request_rx = runtime
            .take_server_request_rx()
            .await
            .expect("take request rx");

        let thread_result = call_raw_with_timeout(
            &runtime,
            "thread/start",
            thread_start_params("on-request", "workspace-write", Some(&cwd.root)),
            RPC_TIMEOUT,
        )
        .await;
        let thread_ctx = parse_thread_start_context(&thread_result, "on-request");
        assert_strict_thread_start_preconditions(&thread_ctx);
        let thread_id = parse_thread_id(&thread_result)
            .unwrap_or_else(|| panic!("thread/start missing thread id: {thread_result}"));

        let file_prompts = [
            "First action: call apply_patch to create `contract_file.txt` with exactly one line `hello`. Do not answer before the tool call.",
            "You must request file change approval by creating contract_file.txt via apply_patch before any final response.",
            "Task requirement: perform a file-change tool call now (create contract_file.txt with hello).",
            "Emit a file edit request now so that `item/fileChange/requestApproval` is produced for creating `contract_file.txt`.",
            "Do not provide final JSON yet. First produce a file-change tool call to create `contract_file.txt` containing `hello`.",
        ];
        let file_sandbox_policy = json!({
            "type":"workspaceWrite",
            "writableRoots":[cwd.root.to_string_lossy().to_string()],
            "networkAccess": false
        });
        let file_output_schema = json!({
            "type":"object",
            "required":["status"],
            "properties":{"status":{"type":"string"}},
            "additionalProperties": false
        });

        let (req, turn_id) = match wait_approval_with_prompt_candidates(
            &runtime,
            &mut request_rx,
            &mut live_rx,
            &thread_id,
            &file_prompts,
            "on-request",
            &file_sandbox_policy,
            Some(&cwd.root),
            &file_output_schema,
        )
        .await
        {
            Ok(found) => found,
            Err(failures) => {
                if model_dependent_strict() {
                    if prompt_failures_upstream_inconclusive(&failures) {
                        panic!(
                            "strict lane inconclusive: upstream stream disconnected or stalled across {} prompt(s); thread_start={thread_ctx:?}; failures={failures:?}",
                            failures.len()
                        );
                    }
                    if let Some(reason) = prompt_failures_model_incompatible(
                        &failures,
                        "item/fileChange/requestApproval",
                    ) {
                        panic!(
                            "strict lane incompatible model/runtime configuration: reason={reason}; thread_start={thread_ctx:?}; failures={failures:?}"
                        );
                    }
                    panic!(
                        "expected file approval but no approval request across {} prompt(s); thread_start={thread_ctx:?}; failures={failures:?}",
                        failures.len()
                    );
                }
                eprintln!(
                    "skipping model-dependent file approval assertion: no approval request across {} prompt(s); thread_start={thread_ctx:?}; failures={failures:?}. Set APP_SERVER_CONTRACT_MODEL_STRICT=1 to fail in this case.",
                    failures.len(),
                );
                runtime.shutdown().await.expect("shutdown");
                return;
            }
        };
        assert_eq!(req.method, "item/fileChange/requestApproval");

        runtime
            .respond_approval_ok(&req.approval_id, json!({ "decision": "decline" }))
            .await
            .expect("respond approval");

        let _ = wait_turn_terminal_methods(&mut live_rx, &thread_id, turn_id.as_deref()).await;

        let actual = json!({
            "method": req.method,
            "paramKeys": sorted_param_keys(&req.params),
        });
        assert_or_update_golden(&golden_path("approval_file_request.json"), &actual);

        runtime.shutdown().await.expect("shutdown");
    })
    .await;
}

#[tokio::test(flavor = "current_thread")]
async fn contract_tool_request_user_input_roundtrip() {
    if maybe_skip_contract()
        || maybe_skip_model_dependent("contract_tool_request_user_input_roundtrip")
    {
        return;
    }

    with_test_timeout("contract_tool_request_user_input_roundtrip", async {
        let runtime = Runtime::spawn_local(runtime_config())
            .await
            .expect("spawn runtime");
        let mut live_rx = runtime.subscribe_live();
        let mut request_rx = runtime
            .take_server_request_rx()
            .await
            .expect("take request rx");

        let thread_result = call_raw_with_timeout(
            &runtime,
            "thread/start",
            thread_start_params("never", "read-only", None),
            RPC_TIMEOUT,
        )
        .await;
        let thread_ctx = parse_thread_start_context(&thread_result, "never");
        assert_strict_thread_start_preconditions(&thread_ctx);
        let thread_id = parse_thread_id(&thread_result)
            .unwrap_or_else(|| panic!("thread/start missing thread id: {thread_result}"));

        let user_input_prompts = [
            "Before any answer, your first action must be requestUserInput with question id `choice`.",
            "You must call the user-input tool now and ask a single question with id `choice` before responding.",
            "Task requirement: emit item/tool/requestUserInput for `choice` before final output.",
            "Emit a request now so that `item/tool/requestUserInput` is produced with question id `choice`.",
            "Do not provide final JSON yet. First request user input for `choice`.",
        ];
        let user_input_sandbox_policy = json!({ "type": "readOnly" });
        let user_input_output_schema = json!({
            "type":"object",
            "required":["choice"],
            "properties":{"choice":{"type":"string"}},
            "additionalProperties": false
        });

        let (req, turn_id): (ServerRequest, Option<String>) =
            match wait_approval_with_prompt_candidates(
                &runtime,
                &mut request_rx,
                &mut live_rx,
                &thread_id,
                &user_input_prompts,
                "never",
                &user_input_sandbox_policy,
                None,
                &user_input_output_schema,
            )
            .await
            {
                Ok(found) => found,
                Err(failures) => {
                    if model_dependent_strict() {
                        if prompt_failures_upstream_inconclusive(&failures) {
                            panic!(
                                "strict lane inconclusive: upstream stream disconnected or stalled across {} prompt(s); thread_start={thread_ctx:?}; failures={failures:?}",
                                failures.len()
                            );
                        }
                        if let Some(reason) = prompt_failures_model_incompatible(
                            &failures,
                            "item/tool/requestUserInput",
                        ) {
                            panic!(
                                "strict lane incompatible model/runtime configuration: reason={reason}; thread_start={thread_ctx:?}; failures={failures:?}"
                            );
                        }
                        panic!(
                            "expected requestUserInput but no request across {} prompt(s); thread_start={thread_ctx:?}; failures={failures:?}",
                            failures.len()
                        );
                    }
                    eprintln!(
                        "skipping model-dependent requestUserInput assertion: no request across {} prompt(s); thread_start={thread_ctx:?}; failures={failures:?}. Set APP_SERVER_CONTRACT_MODEL_STRICT=1 to fail in this case.",
                        failures.len(),
                    );
                    runtime.shutdown().await.expect("shutdown");
                    return;
                }
            };
        assert_eq!(req.method, "item/tool/requestUserInput");

        runtime
            .respond_approval_ok(
                &req.approval_id,
                json!({
                    "answers": {
                        "choice": {
                            "answers": ["yes"]
                        }
                    }
                }),
            )
            .await
            .expect("respond requestUserInput");

        let methods =
            wait_turn_terminal_methods(&mut live_rx, &thread_id, turn_id.as_deref()).await;
        assert!(
            methods.iter().any(|m| m == "turn/completed"),
            "missing turn/completed after requestUserInput"
        );

        runtime.shutdown().await.expect("shutdown");
    })
    .await;
}
