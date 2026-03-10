# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.3.0] - 2026-03-10

### Added
- session-scoped `coclai::automation` module with `AutomationSpec`, `AutomationStatus`, `AutomationHandle`, and `spawn(session, spec)`
- single-flight recurring runner coverage for delayed start, stop-at boundaries, max-runs, same-thread reuse, explicit stop, and closed-session terminal failure
- public automation contract documentation in the README and API reference
- automation design and task docs under `docs/AUTOMATION_PLAN.md` and `docs/AUTOMATION_TASKS.md`

### Changed
- root crate surface now exposes automation as an optional layer above prepared `Session` handles
- release verification now covers automation-specific user scenarios in addition to workspace-wide fmt, clippy, and test gates

## [0.2.2] - 2026-03-09

### Fixed
- real-server approval and pre-tool-use live gates now require approval-gated conditions (`read-only` + `on-request`) instead of permissive write sandboxes
- low-level AppServer approval live gate now validates the core approval bridge contract without depending on assistant completion latency
- session-scoped and run-profile `PreToolUse` hook paths remain covered by deterministic regressions while the live gate stays minimal and stable

### Changed
- release documentation now reflects 9 opt-in real-server scenarios and the narrower approval-hook contract

## [0.2.1] - 2026-03-09

### Added
- typed `skills/list` support
- typed `command/exec`, `command/exec/write`, `command/exec/resize`, `command/exec/terminate`
- initialize capability override with explicit `experimental_api` opt-in
- high-level `output_schema` forwarding
- hook filtering and shell-hook surfaces in the public API

### Changed
- README and API reference rewritten around the layered public API
- `thread/*` sandbox wire uses upstream string `sandbox` mode
- `turn/start` and `command/exec` keep upstream object `sandboxPolicy`
- release verification centered on local preflight scripts

## [0.2.0] - 2026-03-08

### Changed
- Version bump; internal dependency alignment

## [0.1.7] - 2026-03-07

### Fixed
- Integration test `real_server.rs` assertion corrections

## [0.1.6] - 2026-03-06

### Changed
- Metadata sync; crate description and repository fields added

## [0.1.5] - 2026-03-05

### Added
- Initial crates.io publish preparation
- Safe defaults: `approval=never`, `sandbox=read-only`, `effort=medium`, `timeout=120s`

## [0.1.0] - 2026-03-04

### Added
- Initial release: `quick_run`, `Workflow`, `AppServer`, `Runtime` layers
- `WorkflowConfig` builder pattern
- `ClientError`, `RuntimeError`, `PromptRunError` error hierarchy
- Codex version compatibility guard (`DEFAULT_MIN_CODEX_VERSION = 0.104.0`)
