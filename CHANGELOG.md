# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.1] - 2025-03-09

### Added
- `CommandExecApi` with full shell-command execution support via `shell_exec` and `shell_exec_raw`
- `CommandExecOutput`, `ShellExecRequest`, `ShellExecResponse` types
- `Skills` and `Policy` types in `runtime::api::types`
- `thread_resume` and additional thread/turn API helpers
- Expanded `AppServer` contract tests (`validated_calls`)
- 89 new unit tests (282 total, up from 193)

### Changed
- README rewritten with full API reference and edge-state table
- CI workflow removed (replaced by local preflight script)

## [0.2.0] - 2025-03-08

### Changed
- Version bump; internal dependency alignment

## [0.1.7] - 2025-03-07

### Fixed
- Integration test `real_server.rs` assertion corrections

## [0.1.6] - 2025-03-06

### Changed
- Metadata sync; crate description and repository fields added

## [0.1.5] - 2025-03-05

### Added
- Initial crates.io publish preparation
- Safe defaults: `approval=never`, `sandbox=read-only`, `effort=medium`, `timeout=120s`

## [0.1.0] - 2025-03-04

### Added
- Initial release: `quick_run`, `Workflow`, `AppServer`, `Runtime` layers
- `WorkflowConfig` builder pattern
- `ClientError`, `RuntimeError`, `PromptRunError` error hierarchy
- Codex version compatibility guard (`DEFAULT_MIN_CODEX_VERSION = 0.104.0`)
