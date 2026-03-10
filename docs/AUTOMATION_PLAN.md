# Automation Plan

Status: Proposed

Related task table: `docs/AUTOMATION_TASKS.md`

## Goal

Add a minimal automation layer that can drive one prepared `Session` on a schedule.

It must support the core user job:

- start later at one explicit time
- repeat the same prompt at a fixed interval
- stop at one explicit time or after a fixed number of runs
- reuse the same `Session` for every turn

This is enough for "start tonight and keep working through the night" while the same process stays alive.

## Why This Belongs In `coclai`

`coclai` already exposes the right primitives:

- `Client::start_session` and `Client::resume_session` create or restore a reusable thread
- `Session::ask` continues the loaded thread without reopening it
- optional modules already exist at the crate root (`web`, `artifact`)

The missing piece is not another runtime layer. The missing piece is a small orchestration layer above `Session`.

## Decision

Add a new optional top-level module:

- `crates/coclai/src/automation.rs`

Export it from:

- `crates/coclai/src/lib.rs`

Do not add this feature to:

- `Session`
- `Workflow`
- `web`
- `artifact`

## V1 Scope

In scope:

- one new `coclai::automation` module
- one public entrypoint that accepts an existing `Session`
- one typed schedule model based on absolute times and `Duration`
- one background runner task
- one handle for stop, wait, and status
- unit and integration verification
- README and API reference updates

Out of scope:

- slash-command UX such as `/loop`
- cron parsing
- human time parsing inside the crate
- time zone conversion inside the crate
- durable restore after process exit or restart
- automatic daily restarts
- web adapter integration
- artifact persistence
- multiple overlapping turns
- queued backlog replay for every missed tick

## Simplicity Rules

V1 should stay small on purpose.

1. The public API should automate a prepared `Session`, not reimplement session creation.
2. The schedule should accept absolute instants, not cron strings.
3. The runner should allow only one in-flight turn at a time.
4. Missed ticks should collapse into one next eligible run instead of building backlog.
5. No new persistence boundary should be introduced in V1.
6. No new date-time dependency should be added unless a real implementation blocker appears.

## Proposed Public API

```rust
use std::time::{Duration, SystemTime};

pub struct AutomationSpec {
    pub prompt: String,
    pub start_at: Option<SystemTime>,
    pub every: Duration,
    pub stop_at: Option<SystemTime>,
    pub max_runs: Option<u32>,
}

pub struct AutomationStatus {
    pub thread_id: String,
    pub runs_completed: u32,
    pub next_due_at: Option<SystemTime>,
    pub last_started_at: Option<SystemTime>,
    pub last_finished_at: Option<SystemTime>,
    pub state: AutomationState,
    pub last_error: Option<String>,
}

pub enum AutomationState {
    Waiting,
    Running,
    Stopped,
    Failed,
}

pub struct AutomationHandle { /* opaque */ }

impl AutomationHandle {
    pub async fn stop(&self);
    pub async fn wait(self) -> AutomationStatus;
    pub async fn status(&self) -> AutomationStatus;
}

pub fn spawn(session: Session, spec: AutomationSpec) -> AutomationHandle;
```

## Execution Contract

The runner contract should be explicit and small:

- `spawn` starts one background task and returns immediately
- the runner uses the given `Session` for every call
- the runner never creates a second session
- the runner never overlaps turns
- if a scheduled moment arrives while a turn is still running, the runner waits for idle state and executes at most one overdue run
- the runner stops when `stop()` is called, `stop_at` is reached, `max_runs` is exhausted, the session is closed, or any `PromptRunError` is returned by the session turn call

V1 error policy is intentionally strict:

- every `PromptRunError` is terminal for the automation runner
- V1 does not retry, ignore, or downgrade any prompt failure variant
- the terminal status stores the error text in `last_error` and moves to `Failed`

This gives scheduled behavior without turning the crate into a job system.

## Why Absolute Times Instead Of Cron

The user need is "run later tonight and continue overnight", not "support a cron language".

Absolute time boundaries plus a fixed interval are enough:

- `start_at = tonight 22:00`
- `every = 30 minutes`
- `stop_at = tomorrow 06:00`

This keeps the crate aligned with its typed, minimal API style and avoids introducing a parser, time zone rules, and recurring calendar semantics in V1.

If later parity with external schedulers becomes important, a higher layer can translate cron or wall-clock rules into the same runtime contract.

## Internal Design

Public surface:

- keep the public API concrete: `spawn(session: Session, spec: AutomationSpec)`

Internal test seam:

- use a small internal trait for "run one turn on one thread" so schedule behavior can be unit-tested without a live app-server
- implement that trait for `Session`
- keep the trait private to avoid leaking abstraction into the public API

State model:

- shared status snapshot behind one async lock
- stop signal behind one cancellation primitive
- one `tokio::spawn` task owns the loop
- terminal failures are recorded once and stop the loop permanently

Timing model:

- compute the next due time from the schedule contract
- sleep until due
- run one turn
- advance by interval while collapsing missed ticks

## File Plan

Create:

- `crates/coclai/src/automation.rs`
- `docs/AUTOMATION_TASKS.md`

Update:

- `crates/coclai/src/lib.rs`
- `README.md`
- `docs/API_REFERENCE.md`

Keep V1 in one source file unless the implementation becomes harder to read than to split.

## Priority Matrix

Must do now:

- freeze the public contract around `Session`
- implement one single-flight runner
- support delayed start and bounded stop
- make `any PromptRunError stops` part of the module contract
- verify session reuse and stop behavior
- document that the feature is session-scoped and non-durable

Should do now:

- expose a clear status snapshot
- keep error messages readable enough for long-running usage

Can wait:

- cron parsing
- pause and resume
- persistence
- restart recovery
- `web` and `artifact` integration

Will not do in this delivery:

- full Claude Code parity
- process supervisor features
- calendar-grade scheduling semantics

## Critical Path

1. Freeze the boundary and public contract.
2. Implement the runner with one in-flight turn rule.
3. Add verification for schedule timing, stop conditions, and session reuse.
4. Update docs so the contract is visible from the public surface.

## Decision Gates

### Gate 1: Boundary Gate

Check:
- the new API lives outside `Session`, `Workflow`, `web`, and `artifact`

Pass condition:
- one new root-level optional module is enough

On fail:
- stop and reduce scope before any extra integration is added

### Gate 2: Schedule Gate

Check:
- the chosen API can express "start at 22:00, repeat every 30m, stop at 06:00"

Pass condition:
- `start_at + every + stop_at` covers the overnight use case without cron

On fail:
- extend the typed schedule model, not the module placement

### Gate 3: Correctness Gate

Check:
- turns never overlap and the same session thread is reused

Pass condition:
- tests prove one-thread reuse and bounded single-flight execution

On fail:
- hold the feature; do not ship a scheduler that can double-run or fork sessions

### Gate 4: Scope Gate

Check:
- V1 remains non-durable and states that explicitly

Pass condition:
- docs and API wording make the limit clear

On fail:
- fix the contract language before release

## Verification Plan

Unit checks:

- next due time calculation
- delayed start behavior
- stop-at boundary behavior
- max-runs stop behavior
- overdue tick collapse behavior
- any `PromptRunError` moves the runner to `Failed` without retry

Integration checks:

- repeated runs keep the same `thread_id`
- `stop()` ends the loop cleanly
- session close or any prompt failure moves the runner to a terminal state

Documentation checks:

- README describes the feature as optional and session-scoped
- API reference documents non-goals and handle semantics

## Explicit Non-Goal For This Delivery

This work does not make `coclai` a durable overnight worker by itself.

If the user later wants:

- survive terminal exit
- survive process restart
- start automatically every night without a living process

that should be a separate durability design, likely outside this V1 module.
