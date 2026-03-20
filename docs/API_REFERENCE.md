# API Reference

This document fixes the current public surface of `codex-runtime` `0.6.1`.

It focuses on:
- exported Rust entry points
- layer selection
- typed contract boundaries
- validation and security rules

## Design Rules

1. High-level APIs stay small and safe by default.
2. Stable upstream parity lands in typed low-level APIs first.
3. Experimental or custom methods stay available through raw JSON-RPC.
4. Validation is strict on typed paths and only relaxed when callers explicitly choose raw mode.

## Layer Selection

| Layer | Entry point | Typical use |
|-------|-------------|-------------|
| 1 | `quick_run`, `quick_run_with_profile` | one-shot prompt execution |
| 2 | `Workflow`, `WorkflowConfig` | repeated runs in one working directory |
| 3 | `runtime::{Client, Session}` | explicit session lifecycle and typed prompt/session config |
| 4 | `automation::{spawn, AutomationSpec}` | repeated turns on one prepared session |
| 5 | `AppServer` | validated low-level JSON-RPC helper layer |
| 6 | `runtime::Runtime` + raw JSON-RPC | full runtime control, live subscriptions, experimental paths |

## Packaging Model

- one repository
- one published crate
- `web` and `artifact` ship as built-in modules in the default crate
- `quick_run`, `Workflow`, and `automation` are convenience layers above the substrate core

Preferred canonical integration surface for deeper consumers:
- `runtime::{Client, Session, ClientConfig, SessionConfig, RunProfile}`
- `runtime::{PromptRunParams, PromptRunResult, PromptRunError}`
- `runtime::{ServerRequest, ServerRequestConfig}`

Use `AppServer` and `rpc_methods` when low-level parity is required. Use raw JSON-RPC only when the protocol shape is intentionally outside the typed surface.

## Root Crate Surface

`codex_runtime` exports:
- `quick_run`
- `quick_run_with_profile`
- `QuickRunError`
- `Workflow`
- `WorkflowConfig`
- `AppServer`
- `rpc_methods`
- `HookMatcher`
- `FilteredPreHook`
- `FilteredPostHook`
- `ShellCommandHook`
- `automation`
- `artifact`
- `plugin`
- `runtime`
- `web`

## `codex_runtime::runtime`

### Configuration and lifecycle

- `Client`, `ClientConfig`, `ClientError`, `CompatibilityGuard`, `SemVerTriplet`
- `Session`, `SessionConfig`, `RunProfile`
- `Runtime`, `RuntimeConfig`, `InitializeCapabilities`, `RestartPolicy`, `SupervisorConfig`
- `RuntimeHookConfig`, `RuntimeMetricsSnapshot`
- `StdioProcessSpec`, `StdioTransportConfig`
- `ServerRequestRx`

### Prompt, thread, and typed RPC models

- `PromptRunParams`, `PromptRunResult`, `PromptRunError`
- `PromptRunStream`, `PromptRunStreamEvent`
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

### Runtime infrastructure

- `ServerRequest`, `ServerRequestConfig`, `TimeoutAction`
- `RpcError`, `RpcErrorObject`, `RuntimeError`, `SinkError`
- `RpcValidationMode`

Available runtime submodules when direct access is needed:
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

## `codex_runtime::plugin`

Primary traits and types:
- `PreHook`, `PostHook`
- `HookFuture`
- `HookPhase`
- `HookContext`
- `HookAction`
- `BlockReason`
- `HookPatch`
- `HookAttachment`
- `HookIssueClass`
- `HookIssue`
- `HookReport`
- `PluginContractVersion`
- `HookMatcher`, `FilteredPreHook`, `FilteredPostHook`
- `ShellCommandHook`

Contract:
- hooks are phase-scoped and opt-in
- pre-hooks can mutate or block before the next RPC boundary
- post-hooks report outcomes and issues
- plugin compatibility is major-version gated
- tool-use hooks run inside approval-gated command/file-change handling

## `codex_runtime::automation`

Primary types:
- `AutomationSpec`
- `AutomationStatus`
- `AutomationState`
- `AutomationHandle`
- `spawn(session, spec)`

`AutomationSpec` fields:
- `prompt: String`
- `start_at: Option<SystemTime>`
- `every: Duration`
- `stop_at: Option<SystemTime>`
- `max_runs: Option<u32>`

Contract:
- one runner owns one prepared `Session`
- `every` must be greater than zero
- one turn at a time
- due-time backlog collapses to one next eligible run
- any `PromptRunError` is terminal
- no cron parsing, persistence, or restart recovery in v1

## `codex_runtime::web`

Primary types:
- `WebAdapter`, `WebAdapterConfig`
- `RuntimeWebAdapter`, `WebPluginAdapter`, `WebRuntimeStreams`
- `CreateSessionRequest`, `CreateSessionResponse`
- `CreateTurnRequest`, `CreateTurnResponse`
- `CloseSessionResponse`
- `ApprovalResponsePayload`
- `WebError`

Primary functions and methods:
- `WebAdapter::spawn(runtime, config)`
- `WebAdapter::spawn_with_adapter(adapter, config)`
- `create_session(...)`
- `create_turn(...)`
- `close_session(...)`
- `subscribe_session_events(...)`
- `subscribe_session_approvals(...)`
- `post_approval(...)`
- `new_session_id()`
- `serialize_sse_envelope(...)`

Contract:
- bridges runtime sessions into tenant- and session-scoped web flows
- approval responses go back through adapter APIs, not direct runtime state mutation

## `codex_runtime::artifact`

Primary types:
- `ArtifactSessionManager`
- `ArtifactPluginAdapter`, `RuntimeArtifactAdapter`
- `ArtifactSession`
- `ArtifactTaskSpec`, `ArtifactTaskKind`, `ArtifactTaskResult`
- `ArtifactMeta`, `SaveMeta`
- `ArtifactStore`, `FsArtifactStore`
- `DomainError`, `StoreErr`, `PatchConflict`
- `DocPatch`, `ValidatedPatch`

Primary functions and methods:
- `ArtifactSessionManager::new(runtime, store)`
- `ArtifactSessionManager::new_with_adapter(adapter, store)`
- `open(artifact_id)`
- `run_task(spec)`
- `FsArtifactStore::new(root)`
- `compute_revision(...)`
- `validate_doc_patch(...)`
- `apply_doc_patch(...)`

Contract:
- keeps persistent artifact state in an `ArtifactStore`
- delegates prompt execution through an adapter boundary
- checks plugin contract compatibility before artifact tasks run
- keeps patch transforms pure and isolates store/runtime side effects in the manager/adapter layer

## High-Level APIs

### `quick_run(cwd, prompt)`

Role: connect with defaults, run one prompt, shut down immediately.

Success result:
- `PromptRunResult { thread_id, turn_id, assistant_text }`

Failure surface:
- `QuickRunError::Connect`
- `QuickRunError::Run { run, shutdown }`
- `QuickRunError::Shutdown`

### `quick_run_with_profile(cwd, prompt, profile)`

Role: same as `quick_run`, but with one reusable `RunProfile`.

Contract:
- the profile is converted into prompt params and hook configuration before execution
- the helper still owns connect, run, and shutdown lifecycle

### `Workflow`

Role: high-level reusable entry point for repeated runs in one working directory.

Contract:
- wraps a prepared client/session path behind a smaller builder surface
- intentionally does not mirror every low-level runtime field
- use `Client` and `Session` when you need explicit lifecycle control

### `Session`

Primary methods:
- `ask(...)`
- `ask_stream(...)`
- `ask_wait(...)`
- `ask_with(...)`
- `ask_with_profile(...)`
- `interrupt_turn(...)`
- `close(...)`
- `is_closed()`

Streaming contract:
- `ask_stream(...)` returns one scoped `PromptRunStream`
- use `recv().await` for typed turn-scoped live events
- use `finish().await` for the final `PromptRunResult`
- dropping an unfinished stream triggers a best-effort interrupt and cleanup path

## Validation and Security Rules

### Validation

- typed request and response helpers validate shape before exposing structured data
- contract validation stays stricter than raw JSON-RPC by design
- malformed request data is surfaced as `RpcError`
- raw mode is still available for experimental or custom upstream methods

### Sandbox and approval

- default sandbox is `read-only`
- default approval is `never`
- privileged escalation requires explicit opt-in
- privileged sandbox validation is enforced on both thread-start and turn-start typed paths
- tool-use hooks do not replace sandbox or approval policy

### Runtime cleanup and metrics

- detached cleanup work is planned first, then executed, so runtime/no-runtime fallback stays explicit
- helper-runtime initialization failures are tracked in runtime metrics snapshots
- cleanup remains best-effort on stream drop and pending-RPC guard drop

## Change Guidance

When editing this crate:
- keep high-level docs aligned with the exported surface in [`../README.md`](../README.md)
- keep contract details aligned with `runtime::rpc_contract` and typed `runtime::api` models
- update [`TEST_TREE.md`](TEST_TREE.md) when test layering or release gates change
