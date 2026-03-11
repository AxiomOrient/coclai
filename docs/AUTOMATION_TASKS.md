# Automation Tasks

Scope: session-scoped recurring prompt orchestration above `Session`

Plan reference: `docs/AUTOMATION_PLAN.md`

## Task Table

| Task ID | Priority | Action | Done When | Evidence Required | Depends On |
|---|---|---|---|---|---|
| AUTO-01 | P0 | Freeze the public contract for `codekko::automation` around an existing `Session`, absolute start/stop times, fixed interval, a minimal handle, and strict failure semantics. | The API shape, V1 non-goals, execution contract, and the rule `any PromptRunError stops the runner` are written down in code comments or module docs before behavior work starts. | Module-level contract text matches `docs/AUTOMATION_PLAN.md` and does not introduce cron, durability, `Client`-owned session creation, or implicit retry behavior. | None |
| AUTO-02 | P0 | Add `crates/codekko/src/automation.rs` with `AutomationSpec`, `AutomationStatus`, `AutomationState`, `AutomationHandle`, and `spawn(session, spec)`. | The crate exposes one root-level automation module and the public API builds without touching `Session` or `Workflow` signatures. | `cargo test` or `cargo check` passes for the new module shape; diff shows only the new module and root export for the public surface. | AUTO-01 |
| AUTO-03 | P0 | Implement the single-flight runner loop with delayed start, fixed interval, stop-at, max-runs, stop signal, overdue tick collapse, and strict failure stop behavior. | One background task can drive repeated `Session::ask` calls without overlap and reaches a terminal state on stop, closed session, or any `PromptRunError`. | Focused tests or deterministic fakes prove no overlap, bounded stop, overdue tick collapse, and that any `PromptRunError` moves the runner to `Failed` without retry. | AUTO-02 |
| AUTO-04 | P1 | Add verification coverage for scheduling math and session reuse. | The smallest useful test set covers delayed start, stop-at, max-runs, and same-thread reuse across repeated turns. | Unit tests for schedule/state transitions and one integration-style test proving repeated runs keep the same `thread_id`. | AUTO-03 |
| AUTO-05 | P1 | Update public docs in `README.md` and `docs/API_REFERENCE.md`. | The docs show the feature as optional, session-scoped, non-durable, and driven by a prepared `Session`. | README example and API reference contract both match the shipped API and explicitly exclude cron and restart persistence. | AUTO-03 |
| AUTO-06 | P1 | Run the release-quality verification path for the new surface. | The feature passes the crate's normal formatting, lint, and test gates, or any skipped gate is explicitly recorded with the reason. | Fresh command results for `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test --workspace`. | AUTO-04, AUTO-05 |

## Execution Notes

- The shortest path is `AUTO-01 -> AUTO-02 -> AUTO-03 -> AUTO-04 -> AUTO-05 -> AUTO-06`.
- Do not add cron parsing to unblock AUTO-03.
- Do not add persistence to unblock AUTO-03.
- If testability becomes awkward, add one private internal trait for turn execution instead of widening the public API.

## Stop Conditions

Stop the implementation and re-evaluate if any of these become necessary for V1:

- modifying `Session` public methods
- adding a new date-time crate only for cron or local-time parsing
- adding storage or restart recovery
- integrating with `web` or `artifact`
