| ID | Status | Goal | Scope | Verification |
|----|----|----|----|----|
| T1 | done | Expose per-client process launch settings without breaking the `codex app-server` invariant | Added `process_env`, `process_cwd`, `app_server_args` and targeted builders in `runtime/client/config.rs`; wired them into `Client::connect()` in `runtime/client/mod.rs` | `config_builder_sets_fields`, `connect_forwards_process_launch_settings_to_app_server_child`, `cargo test --workspace` |
| T2 | done | Add a scoped streaming helper on the canonical session bridge | Added `Session::ask_stream(...)`, `PromptRunStream`, and typed per-turn delivery built on the existing collector/lag fallback path | `session_ask_stream_yields_scoped_events_and_final_result`, `session_ask_stream_finishes_with_turn_failure_context`, `cargo test --workspace` |
| T3 | done | Expand typed live-event extraction where raw JSON parsing is common and repetitive | Added phase-1 typed extractors for `item/agentMessage/delta`, `turn/completed`, and `turn/failed` in `runtime/events.rs` | `detects_agent_message_delta_notification`, `detects_turn_completed_notification`, `detects_turn_failed_notification`, `cargo test --workspace` |
| T4 | done | Align public docs and examples with the new canonical bridge surface | Updated `README.md` and `docs/API_REFERENCE.md` for launch controls, scoped session streaming, and typed live extractor coverage | Doc changes present and `cargo test --workspace` passed after edits |
| T5 | done | Reduce prompt-run internal statefulness by centralizing pre-turn hook preparation and stream cleanup context | Added a shared pre-turn hook preparation helper in `runtime/api/prompt_run.rs` and grouped stream state/cleanup data in `runtime/api/models.rs` | `prompt_stream_drop_runs_post_turn_hooks`, `cargo test --workspace` |
| T6 | done | Reduce prompt stream observation branching to one pure reduction step | Added a pure stream observation reducer and terminal-result builder in `runtime/api/prompt_run.rs` so `recv()` only handles I/O and cleanup dispatch | `session_ask_stream_yields_scoped_events_and_final_result`, `prompt_stream_drop_runs_post_turn_hooks`, `cargo test --workspace` |

Resolved build decisions:
- `ClientConfig` kept targeted launch builders only; no `with_process_spec(...)` passthrough was added.
- The first scoped streaming API shipped on `Session` as `ask_stream(...)`; no parallel `Runtime` helper was added in this slice.
- Approval live-event extractors stayed out of phase 1; the canonical approval path remains `ServerRequest`.
- `PromptRunResult` stayed lean; richer terminal state continues to flow through the new stream helper plus existing `thread_read` escape hatch.
