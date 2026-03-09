# API_REFERENCE

`coclai` exposes a layered API around the local `codex app-server`.

This document fixes the public API boundary, the typed payload contracts, and the validation and security rules.

## Design Rules

1. High-level APIs are easy to use and intentionally small.
2. Stable upstream parity goes to low-level typed APIs first.
3. Experimental or custom methods stay available through raw JSON-RPC.
4. Validation is strict by default and opt-out only when the caller chooses raw mode.

## Layer Selection Guide

Choose the narrowest surface that solves the job.

| Layer | Entry point | Typical use |
|-------|-------------|-------------|
| 1 | `quick_run`, `quick_run_with_profile` | One-shot usage with safe defaults |
| 2 | `Workflow`, `WorkflowConfig` | Repeated runs with a shared working directory and profile defaults |
| 3 | `runtime::{Client, Session}` | Explicit session lifecycle and typed run/session configuration |
| 4 | `AppServer` | Thin JSON-RPC facade with validated request helpers and server-request loop access |
| 5 | `runtime::Runtime` + raw JSON-RPC | Full runtime control, live events, raw/validated RPC; raw mode for experimental or custom methods |

## Public Surface Map

### Root crate (`coclai`)
- `quick_run`
- `quick_run_with_profile`
- `QuickRunError`
- `Workflow`
- `WorkflowConfig`
- `AppServer`
- `rpc_methods` (re-export of `runtime::rpc_contract::methods`)
- `web` (optional)
- `artifact` (optional)
- `plugin`
- `runtime`

### `coclai::runtime` re-exports

Configuration and lifecycle:
- `Client`, `ClientConfig`, `CompatibilityGuard`, `SemVerTriplet`
- `Session`, `SessionConfig`, `RunProfile`
- `Runtime`, `RuntimeConfig`, `InitializeCapabilities`, `RestartPolicy`, `SupervisorConfig`
- `RuntimeHookConfig`, `RuntimeMetricsSnapshot`
- `StdioProcessSpec`, `StdioTransportConfig`
- `ServerRequestRx` (type alias: `tokio::sync::mpsc::Receiver<ServerRequest>`)

Typed API models:
- `PromptRunParams`, `PromptRunResult`, `PromptRunError`
- `ThreadStartParams`, `TurnStartParams`, `ThreadHandle`, `TurnHandle`
- `ThreadReadParams`, `ThreadReadResponse`
- `ThreadListParams`, `ThreadListResponse`, `ThreadListSortKey`
- `ThreadLoadedListParams`, `ThreadLoadedListResponse`
- `ThreadRollbackParams`, `ThreadRollbackResponse`
- `ThreadView`, `ThreadTurnView`, `ThreadTurnErrorView`, `ThreadItemView`, `ThreadItemPayloadView`
- `ThreadTurnStatus`, `ThreadItemType`, `ThreadAgentMessageItemView`, `ThreadCommandExecutionItemView`
- `SkillsListParams`, `SkillsListResponse`, `SkillsListEntry`, `SkillsListExtraRootsForCwd`
- `SkillMetadata`, `SkillInterface`, `SkillDependencies`, `SkillToolDependency`, `SkillErrorInfo`, `SkillScope`
- `CommandExecParams`, `CommandExecResponse`
- `CommandExecWriteParams`, `CommandExecWriteResponse`
- `CommandExecResizeParams`, `CommandExecResizeResponse`
- `CommandExecTerminateParams`, `CommandExecTerminateResponse`
- `CommandExecOutputDeltaNotification`, `CommandExecOutputStream`, `CommandExecTerminalSize`
- `PromptAttachment`, `InputItem`, `ByteRange`, `TextElement`
- `ApprovalPolicy`, `SandboxPolicy`, `SandboxPreset`, `ExternalNetworkAccess`
- `ReasoningEffort`, `ServiceTier`, `Personality`
- `DEFAULT_REASONING_EFFORT`

Runtime infrastructure:
- `ServerRequest`, `ServerRequestConfig`, `TimeoutAction`
- `RpcError`, `RpcErrorObject`, `RuntimeError`, `SinkError`
- `RpcValidationMode`

### `coclai::plugin`

Traits and types:
- `PreHook`, `PostHook` — async lifecycle extension traits
- `HookFuture` — pinned boxed future type alias for hook return values
- `HookPhase` — `PreRun`, `PostRun`, `PreSessionStart`, `PostSessionStart`, `PreTurn`, `PostTurn`, `PreToolUse`, `PostToolUse`
- `HookContext` — phase, thread/turn ids, cwd, model, correlation id, metadata
- `HookAction` — `Noop`, `Mutate(HookPatch)`, or `Block(BlockReason)`
- `BlockReason` — explicit pre-hook deny reason
- `HookPatch` — `prompt_override`, `model_override`, `add_attachments`, `metadata_delta`
- `HookAttachment` — `AtPath`, `ImageUrl`, `LocalImage`, `Skill`
- `HookIssueClass` — `Validation`, `Execution`, `Timeout`, `Internal`
- `HookIssue` — structured hook failure record
- `HookReport` — accumulated hook issues for one call
- `PluginContractVersion` — major-version compatibility check
- `HookMatcher`, `FilteredPreHook`, `FilteredPostHook` — pure filtering wrappers
- `ShellCommandHook` — external `sh -c` adapter for pre/post hooks

Contract:
- Hooks are phase-scoped and opt-in.
- Plugin compatibility is major-version gated (`PluginContractVersion::is_compatible_with`).
- Hook issues are recorded in `HookReport` instead of silently discarded.
- A pre-hook returning `HookAction::Mutate` can override prompt, model, and add attachments.
- A pre-hook returning `HookAction::Block` stops the call before the next RPC boundary.
- Tool-use hooks are routed through the internal approval loop and fire for approval-gated tool/file-change requests.

### `coclai::web`

Primary types:
- `WebAdapter`, `WebAdapterConfig`
- `CreateSessionRequest`, `CreateSessionResponse`
- `CreateTurnRequest`, `CreateTurnResponse`
- `CloseSessionResponse`
- `ApprovalResponsePayload`
- `WebError`
- `new_session_id()`
- `serialize_sse_envelope(...)`

Contract:
- This module bridges runtime sessions into web-facing session and approval flows.
- It is multi-tenant by explicit `tenant_id` and `session_id` boundaries.
- Approval responses are posted back through the adapter, not by mutating runtime state directly.

### Public runtime submodules

Available for direct use when the re-export set is not enough:
- `runtime::api`
- `runtime::approvals`
- `runtime::client`
- `runtime::core`
- `runtime::errors`
- `runtime::events`
- `runtime::hooks`
- `runtime::metrics`
- `runtime::rpc`
- `runtime::rpc_contract`
- `runtime::sink`
- `runtime::state`
- `runtime::transport`
- `runtime::turn_output`

## High-Level API

### `quick_run(cwd, prompt)`

Role: connect with default config → run one prompt → shutdown immediately.

Success result: `PromptRunResult { thread_id, turn_id, assistant_text }`

Failure surface:
- `QuickRunError::Connect` — child process or handshake failed
- `QuickRunError::Run { run, shutdown }` — prompt failed; shutdown error attached if it also failed
- `QuickRunError::Shutdown` — prompt succeeded but shutdown failed

### `quick_run_with_profile(cwd, prompt, profile)`

Same lifecycle as `quick_run` with explicit control over:
- model, effort, approval policy, sandbox policy
- privileged escalation approval
- attachments, timeout, output schema, run hooks

### `Workflow`

Methods:
- `connect(config)` — connect once with one explicit workflow config
- `connect_default(cwd)` — connect with defaults for one cwd
- `run(prompt)` — run one prompt using workflow defaults
- `run_with_profile(prompt, profile)` — run one prompt with explicit profile override
- `setup_session()` — start one session using workflow defaults
- `setup_session_with_profile(profile)` — start one session with explicit profile override
- `config()` — borrow the workflow config
- `client()` — borrow the underlying client
- `shutdown()` — explicit shutdown

Contract:
- `WorkflowConfig` stores both connect-time client config and run-time profile defaults.
- Repeated runs and session setup share the same cwd and profile baseline.

### `WorkflowConfig`

Client-level builders (affect the connect phase):
- `with_cli_bin` — override codex binary location
- `with_compatibility_guard` — override runtime compatibility policy
- `without_compatibility_guard` — disable compatibility guard
- `with_initialize_capabilities` — override initialize capability switches
- `enable_experimental_api` — opt into Codex experimental app-server methods and fields
- `with_global_hooks`, `with_global_pre_hook`, `with_global_post_hook` — register hooks for the entire runtime lifetime
- `with_global_pre_tool_use_hook` — register approval-loop tool interception hooks
- `with_shell_pre_hook`, `with_shell_post_hook`, `with_shell_pre_hook_timeout` — register shell-backed global hooks

Run-level builders (affect each prompt or session call):
- `with_model`, `with_effort`, `with_approval_policy`, `with_sandbox_policy`, `with_timeout`
- `with_output_schema` — JSON Schema for the final assistant message
- `with_attachment`, `attach_path`, `attach_path_with_placeholder`, `attach_image_url`, `attach_local_image`, `attach_skill`
- `with_run_hooks`, `with_run_pre_hook`, `with_run_post_hook` — register hooks scoped to each run

Conversion:
- `to_session_config()` — build a `SessionConfig` from cwd and profile defaults

Contract:
- This is the simplest reusable configuration object.
- It does not attempt to expose every low-level upstream field.

## Client And Session API

### `Client`

Methods:
- `connect_default()` — connect using default config (default CLI)
- `connect(config)` — connect using explicit client config
- `run(cwd, prompt)` — run one prompt with default policies
- `run_with(params)` — run one prompt with explicit `PromptRunParams`
- `run_with_profile(cwd, prompt, profile)` — run one prompt with a reusable profile
- `start_session(config)` — start a prepared session and return a reusable handle
- `resume_session(thread_id, config)` — resume an existing thread id with prepared defaults
- `runtime()` — borrow underlying runtime for full low-level control
- `config()` — return connect-time client config snapshot
- `shutdown()` — shutdown child process and background tasks

Contract:
- `connect()` spawns `codex app-server` as a child process.
- Initialize compatibility is checked unless disabled in config.
- `resume_session()` performs `thread/resume` once; `Session::ask*` reuses the already-loaded thread path without a second resume.

### `ClientConfig`

Fields: `cli_bin`, `compatibility_guard`, `initialize_capabilities`, `hooks`

Key builders:
- `with_cli_bin(...)`, `with_compatibility_guard(...)`, `without_compatibility_guard()`
- `with_initialize_capabilities(...)`, `enable_experimental_api()`
- `with_hooks(...)`, `with_pre_hook(...)`, `with_post_hook(...)`, `with_pre_tool_use_hook(...)`

Default CLI binary: `codex` (resolved via `PATH`).

### `InitializeCapabilities`

Typed initialize capability override. Currently supported:
- `experimental_api: bool`

Contract:
- High-level APIs do not expose arbitrary initialize payload mutation.
- Stable capability toggles are surfaced here; experimental opt-in is explicit.

### `RunProfile`

Default values:

| Field | Default |
|-------|---------|
| `model` | `None` (server default) |
| `effort` | `medium` |
| `approval_policy` | `never` |
| `sandbox_policy` | `read-only` |
| `privileged_escalation_approved` | `false` |
| `attachments` | `[]` |
| `timeout` | `120s` |
| `output_schema` | `None` |
| `hooks` | empty |

Hook builders:
- `with_hooks(...)`, `with_pre_hook(...)`, `with_post_hook(...)`
- `with_pre_tool_use_hook(...)`
- `allow_privileged_escalation()`

### `SessionConfig`

Role: bundle `cwd + RunProfile` as reusable session defaults.

Methods:
- `new(cwd)` — create with safe defaults
- `from_profile(cwd, profile)` — create from an existing profile
- `profile()` — materialize a `RunProfile` view of the session defaults
- Plus the same builder set as `RunProfile`

Note: `cwd` can only be set at construction time. To change cwd, create a new `SessionConfig` via `from_profile(new_cwd, config.profile())`.

### `Session`

Methods:
- `is_closed()` — check whether the session has been closed
- `ask(prompt)` — run one turn with session defaults
- `ask_with(params)` — run one turn with explicit `TurnStartParams`
- `ask_with_profile(prompt, profile)` — run one turn with a profile override
- `profile()` — borrow the session config
- `interrupt_turn(turn_id)` — interrupt the current turn
- `close()` — archive thread and mark session closed

Contract:
1. `close()` is single-flight and reuses the first archive result.
2. Closed sessions reject new prompt or RPC actions locally without sending any requests.
3. Loaded sessions do not send a second `thread/resume` for `ask*`.

## AppServer API

### Connection
- `AppServer::connect(config)` — connect with explicit config
- `AppServer::connect_default()` — connect with default runtime discovery

### Validated request and notify

These enforce known request and response shapes by default (`RpcValidationMode::KnownMethods`):
- `request_json(method, params)` → `Result<Value, RpcError>`
- `request_json_with_mode(method, params, mode)` → `Result<Value, RpcError>`
- `request_typed<P, R>(method, params)` → `Result<R, RpcError>`
- `request_typed_with_mode<P, R>(method, params, mode)` → `Result<R, RpcError>`
- `notify_json(method, params)` → `Result<(), RuntimeError>`
- `notify_json_with_mode(method, params, mode)` → `Result<(), RuntimeError>`
- `notify_typed<P>(method, params)` → `Result<(), RuntimeError>`
- `notify_typed_with_mode<P>(method, params, mode)` → `Result<(), RuntimeError>`

### Typed low-level helpers
- `skills_list(params)` — typed `skills/list`
- `command_exec(params)` — typed `command/exec`
- `command_exec_write(params)` — typed `command/exec/write`
- `command_exec_resize(params)` — typed `command/exec/resize`
- `command_exec_terminate(params)` — typed `command/exec/terminate`

### Server-request loop
- `take_server_requests()` → `Result<ServerRequestRx, RuntimeError>` — take exclusive receiver (call once)
- `respond_server_request_ok(approval_id, result)` — reply success to one server request
- `respond_server_request_err(approval_id, err)` — reply error to one server request

### Escape hatches and accessors
- `request_json_unchecked(method, params)` — bypass contract checks (use for experimental/custom methods)
- `notify_json_unchecked(method, params)` — bypass contract checks
- `runtime()` — borrow server runtime for full low-level control
- `client()` — borrow underlying client
- `shutdown()` — explicit shutdown

Contract:
- Validated methods enforce known request and response shapes by default.
- Unchecked methods are the canonical path for experimental or custom RPC calls.
- Approval, request-user-input, and tool-call workflows must consume `take_server_requests()`.

## Runtime API

### Lifecycle and observability
- `spawn_local(config)` — spawn process and initialize
- `subscribe_live()` → `broadcast::Receiver<Envelope>` — subscribe to all live events
- `is_initialized()` — check initialization state
- `state_snapshot()` — latest `RuntimeState` snapshot (threads, turns, server requests)
- `initialize_result_snapshot()` — raw initialize response payload
- `server_user_agent()` — user agent string from initialize response
- `metrics_snapshot()` — `RuntimeMetricsSnapshot`
- `hook_report_snapshot()` — latest `HookReport` from the last hook-enabled call
- `register_hooks(hooks)` — register additional hooks into running runtime (dedup by name)
- `shutdown()` — shutdown child process and background tasks

### Raw and validated RPC
- `call_raw(method, params)` — no validation
- `call_validated(method, params)` — validates with `KnownMethods` mode
- `call_validated_with_mode(method, params, mode)` — explicit validation mode
- `call_typed_validated<P, R>(method, params)` — typed with `KnownMethods` validation
- `call_typed_validated_with_mode<P, R>(method, params, mode)` — typed with explicit mode
- `notify_raw(method, params)` — fire-and-forget, no validation
- `notify_validated(method, params)` — validates with `KnownMethods` mode
- `notify_validated_with_mode(method, params, mode)` — explicit validation mode
- `notify_typed_validated<P>(method, params)` — typed with `KnownMethods` validation
- `notify_typed_validated_with_mode<P>(method, params, mode)` — typed with explicit mode

### Typed thread, turn, skill, and command helpers

Thread and session:
- `thread_start(params)`, `thread_resume(thread_id, params)`
- `thread_fork(thread_id)`, `thread_archive(thread_id)`
- `thread_read(params)`, `thread_list(params)`, `thread_loaded_list(params)`, `thread_rollback(params)`
- `skills_list(params)`

Turn control:
- `ThreadHandle::turn_start(params)`
- `ThreadHandle::turn_steer(expected_turn_id, input)`
- `ThreadHandle::turn_interrupt(turn_id)`
- `Runtime::turn_interrupt(thread_id, turn_id)`
- `Runtime::turn_interrupt_with_timeout(thread_id, turn_id, timeout)`

Command execution:
- `command_exec(params)`, `command_exec_write(params)`, `command_exec_resize(params)`, `command_exec_terminate(params)`

Prompt helpers:
- `run_prompt(params)`, `run_prompt_simple(cwd, prompt)`, `run_prompt_with_hooks(params, hooks)`

## Typed Payload Contracts

### `PromptRunParams`

Fields:
- `cwd`, `prompt`
- Optional overrides: `model`, `effort`, `output_schema`
- Policy: `approval_policy`, `sandbox_policy`, `privileged_escalation_approved`
- `attachments`, `timeout`

Attachment variants:
- `AtPath { path, placeholder }` — file or directory reference
- `ImageUrl { url }` — remote image
- `LocalImage { path }` — local image file
- `Skill { name, path }` — skill definition file

Contract:
1. Default effort is `medium`.
2. Attachment paths are validated before execution (absolute or relative to cwd).
3. Timeout is enforced as a bounded deadline.
4. `output_schema` constrains the shape of the final assistant message.

### `ThreadStartParams`

Stable typed fields:
- `model`, `model_provider`
- `service_tier` (supports explicit null)
- `cwd`
- `approval_policy`, `sandbox_policy`, `privileged_escalation_approved`
- `config`, `service_name`, `base_instructions`, `developer_instructions`
- `personality`, `ephemeral`

Contract:
1. `thread/start` uses upstream `sandbox` key on the wire (not `sandboxPolicy`).
2. `thread/resume` only accepts the shared stable override subset (no start-only fields).
3. `service_name` and `ephemeral` are start-only fields.
4. Experimental fields stay out of the typed surface (see Experimental Field Policy).

### `TurnStartParams`

Stable typed fields:
- `input`, `cwd`
- `approval_policy`, `sandbox_policy`, `privileged_escalation_approved`
- `model`, `service_tier` (supports explicit null)
- `effort`, `summary`, `personality`, `output_schema`

Contract:
1. `input` must not be empty.
2. `turn/start` uses upstream `sandboxPolicy` key on the wire.
3. Experimental `collaborationMode` stays raw-only.

### `SkillsListParams` and `SkillsListResponse`

Request fields:
- `cwds` — list of working directories to search
- `force_reload` — bypass cache
- `per_cwd_extra_user_roots` (`Vec<SkillsListExtraRootsForCwd>`) — extra roots per cwd

Response structure:
- `data: Vec<SkillsListEntry>`
- Each entry: `cwd`, `skills: Vec<SkillsListEntry>`, `errors`

Contract:
- This is typed parity for repo-local skill inventory lookup.
- Live invalidation is represented by the `skills/changed` notification.

### `CommandExecParams`

Fields:
- `command` — executable + args (required, non-empty)
- `process_id` — required when `tty` or streaming is enabled
- Streaming and tty flags: `tty`, `stream_stdin`, `stream_stdout_stderr`
- Caps and timeouts: `output_bytes_cap`, `disable_output_cap`, `disable_timeout`, `timeout_ms`
- Scope: `cwd`, `env`, `size`, `sandbox_policy`

Follow-up payloads:
- `CommandExecWriteParams { process_id, delta_base64, close_stdin }`
- `CommandExecResizeParams { process_id, size }`
- `CommandExecTerminateParams { process_id }`
- `CommandExecOutputDeltaNotification { process_id, stream, delta_base64, cap_reached }`

Validation rules:
1. `command` must not be empty.
2. `tty` or streaming requires `process_id`.
3. `size` is only valid when `tty = true`.
4. `disable_output_cap` and `output_bytes_cap` are mutually exclusive.
5. `disable_timeout` and `timeout_ms` are mutually exclusive.
6. `write` requires at least one of `delta_base64` or `close_stdin`.
7. `timeout_ms` must be >= 0; `output_bytes_cap` must be > 0.

## Events And Server Requests

### Live event stream

- `Runtime::subscribe_live()` returns `broadcast::Receiver<Envelope>`
- Typed helper extraction:
  - `extract_skills_changed_notification(&Envelope)`
  - `extract_command_exec_output_delta(&Envelope)`

### Server-request routing

`ServerRequestConfig` defaults:
- `default_timeout_ms = 30000`
- `on_timeout = Decline`
- `auto_decline_unknown = true`

Known queued methods:
- `item/commandExecution/requestApproval`
- `item/fileChange/requestApproval`
- `item/tool/requestUserInput`
- `item/tool/call`
- `account/chatgptAuthTokens/refresh`

Contract:
- Unknown server requests are auto-declined by default.
- Explicit handling consumes the request queue via `AppServer::take_server_requests()` and replies with `respond_server_request_ok` or `respond_server_request_err`.

## Validation And Security Contracts

### RPC validation mode

| Mode | Behavior |
|------|----------|
| `KnownMethods` (default) | Enforces request and response shape for all 15 known methods |
| `None` | Skips contract validation entirely; intended only for raw or experimental usage |

### Privileged sandbox escalation (SEC-004)

High-risk sandbox execution requires all of the following:
1. `privileged_escalation_approved = true`
2. Approval policy is not `never`
3. Execution scope is explicit through `cwd` or non-empty writable roots

This rule is applied consistently to: `thread/start`, `thread/resume`, `turn/start`.

### Hook-specific contracts

1. Pre/post hook execution failures are fail-open and recorded in `HookReport`.
2. `HookAction::Block` is fail-closed and returns `PromptRunError::BlockedByHook`.
3. Pre-tool-use hooks are observable only when Codex emits approval requests for a tool or file action.
4. Registering pre-tool-use hooks does not remove sandbox or approval constraints; privileged writes still require explicit escalation opt-in.

### Canonical parsing and redaction

- Thread id and turn id accept only canonical fields (`thread.id`, `turn.id`).
- Invalid response payloads are summarized (keys only, no values), never dumped verbatim.
- Child stderr tail is captured but not exposed in public error messages.

## Experimental Field Policy

The following upstream fields are intentionally excluded from typed parity:

| Method | Excluded field | Reason |
|--------|---------------|--------|
| `thread/start` | `dynamicTools` | Unstable protocol |
| `thread/start` | `experimentalRawEvents` | Unstable protocol |
| `thread/start` | `persistExtendedHistory` | Unstable protocol |
| `thread/resume` | `persistExtendedHistory` | Unstable protocol |
| `turn/start` | `collaborationMode` | Unstable protocol |

Use raw RPC when these are required:
- `AppServer::request_json_unchecked`, `AppServer::notify_json_unchecked`
- `Runtime::call_raw`, `Runtime::notify_raw`

Promotion rule: a field moves into typed parity only after upstream stability and testability are confirmed.

## Real-Server Verification Boundary

Deterministic default gates:
- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- `./scripts/check_blocker_regressions.sh`
- `./scripts/check_security_gate.sh`
- `./scripts/check_product_hygiene.sh`

Opt-in real-server gate (9 ignored scenarios):
```bash
COCLAI_REAL_SERVER_APPROVED=1 \
COCLAI_RELEASE_INCLUDE_REAL_SERVER=1 \
./scripts/release_preflight.sh
```

Current live coverage:
- `quick_run`
- `workflow.run`
- `quick_run_with_profile` with attachment
- `workflow.setup_session -> ask`
- `client.resume_session -> ask`
- low-level `AppServer` thread roundtrip
- `AppServer` approval roundtrip

Currently outside the live gate:
- `skills/list`, `command/exec*`
- Extended thread and turn overrides
- `requestUserInput`, dynamic tool-call

Reason: either deterministic live triggering is not stable enough, or the live signal is weaker than unit, contract, and mock integration coverage.
