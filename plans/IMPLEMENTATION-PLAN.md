# AxiomRunner API Gap Review And Bounded Plan

## Scope Contract

- Request: review the proposed `codex-runtime` API improvements for AxiomRunner/AxiomMaker integration and produce a build-ready implementation plan only if the additions fit the current substrate design.
- Target scope: `crates/codex-runtime/src/runtime/{client,api,events,transport}` plus `README.md` and `docs/API_REFERENCE.md`.
- Done condition:
  - each proposal is classified as accept / narrow-accept / defer with design reasons
  - accepted work stays on the canonical `runtime::{Client, Session}` bridge and does not collapse layers into `AppServer`
  - task rows exist for an implementation pass with concrete verification gates

## Neutral Review Outcome

### 1. `ClientConfig` process `env/cwd/app-server args`

Verdict: accept, with a narrower API than `with_process_spec(...)`.

Why this fits:
- child-process startup and transport configuration are explicitly owned by the substrate
- `transport::StdioProcessSpec` already supports `env` and `cwd`
- the current `Client::connect()` hard-codes `app-server` launch details, forcing callers into global environment mutation for per-runtime settings
- `ClientConfig` is already part of the documented canonical AxiomRunner bridge

Constraint:
- avoid exposing an arbitrary `StdioProcessSpec` on `ClientConfig`
- `Client` should still mean "spawn the configured Codex CLI in app-server mode", not "run any child process"

Recommended shape:
- add `process_env: HashMap<String, String>`
- add `process_cwd: Option<PathBuf>`
- add `app_server_args: Vec<String>` for extra args appended after the fixed `app-server` subcommand
- add targeted builders:
  - `with_process_envs(...)`
  - `with_process_env(...)`
  - `with_process_cwd(...)`
  - `with_app_server_args(...)`
  - optional `with_app_server_arg(...)`

### 2. Session/turn scoped streaming API

Verdict: accept, but keep it session-first and helper-shaped.

Why this fits:
- `run_prompt` already implements scoped live collection internally with `TurnStreamCollector`
- current public streaming requires every consumer to subscribe globally and reimplement filtering, terminal detection, and lag handling
- README/SPEC position `runtime::{Client, Session}` as the canonical integration bridge for AxiomRunner, so this boilerplate belongs here rather than in every consumer

Constraint:
- do not replace `Runtime::subscribe_live()`; keep it as the raw/full-control escape hatch
- do not jump straight to a global `Runtime::run_prompt_stream(...)` if the primary consumer need is prepared-session continuation

Recommended first slice:
- add `Session::ask_stream(...)`
- return a scoped handle that owns:
  - `thread_id`
  - `turn_id`
  - a typed event receiver limited to that turn
  - `finish().await -> Result<PromptRunResult, PromptRunError>`
- implement it by reusing the existing turn-start + collector logic rather than inventing a second completion path

### 3. Typed live event extractors

Verdict: accept partially, with priority on stream-only pain points.

Why this fits:
- the repository contract explicitly prefers stable typed parity for recurring upstream shapes
- today only `skills/changed` and `command/exec/outputDelta` have helper extraction, while prompt-stream consumers still decode common turn events by hand

Constraint:
- approval requests already have a typed queued surface via `ServerRequest`
- live-event extractors for approvals may still be convenient, but they should not become the canonical approval integration path

Recommended priority:
- phase 1 typed extractors:
  - `AgentMessageDeltaNotification`
  - `TurnCompletedNotification`
  - `TurnFailedNotification`
- phase 2 convenience extractors, only if needed after phase 1:
  - `FileChangeRequestApprovalNotification`
  - `CommandExecutionRequestApprovalNotification`

### 4. Expanding `PromptRunResult`

Verdict: defer in the first slice.

Why defer:
- the current type is intentionally lean and stable
- widening it to embed `ThreadTurnView` or thread snapshots changes the cost and semantics of every non-streaming prompt helper
- the immediate integration pain is better addressed by the scoped streaming handle and stronger typed event extraction

Preferred follow-up if still needed later:
- add a new opt-in result shape or a dedicated helper such as `thread_read_last_turn(...)`
- avoid silently turning `run_prompt` into a heavier read-after-write API

## Design Summary

1. Extend `ClientConfig` only with process-launch fields that preserve the `codex app-server` invariant.
2. Add one public scoped streaming helper on `Session`, implemented on top of the existing internal turn collection machinery.
3. Add typed extractors for the high-frequency live events that are currently parsed via raw JSON in downstream integrations.
4. Keep `PromptRunResult` unchanged in this slice; let the new streaming handle and existing `thread_read` cover richer output needs.

## Expanded Atomic Path

1. `$scout-boundaries`
2. `$plan-what-it-does`
3. `$plan-how-to-build`
4. `$plan-task-breakdown`

## Build Notes

- Preserve backward compatibility for `Client::connect_default()` and existing `ClientConfig::new()`.
- Keep default launch behavior identical: `codex app-server` with no extra env/cwd overrides.
- Avoid broadening convenience layers; the new streaming API belongs under `runtime::{Client, Session}`.
- Document approval-extractor helpers, if added, as convenience over live events rather than a replacement for `ServerRequest`.

## Verification Gates

- unit tests for `ClientConfig` builder behavior and constructed `StdioProcessSpec`
- unit tests for scoped stream filtering, terminal completion, timeout, and lagged thread-read fallback
- unit tests for each new event extractor against representative envelopes
- docs update in `README.md` and `docs/API_REFERENCE.md` for any newly public API
