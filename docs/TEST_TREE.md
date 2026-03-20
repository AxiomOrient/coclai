# Test Tree

This document explains how tests are grouped and what belongs in each layer.

## Goals

- keep test intent easy to read
- avoid duplicating the same invariant across multiple layers
- keep real-server coverage opt-in and outside the default release gate

## Layers

### `unit`

Use for:
- pure transforms
- model rules
- serialization and data-shape checks
- data-first helpers and validation decisions

Do not use for:
- external process wiring
- network or stdio orchestration
- full runtime lifecycle behavior

### `contract`

Use for:
- JSON-RPC shape validation
- typed helper request and response boundaries
- security and ownership invariants
- compatibility guards and public protocol expectations

### `integration`

Use for:
- cross-module lifecycle behavior
- runtime wiring through mock runtime or process abstractions
- approval and streaming flow behavior
- session, thread, and artifact orchestration

## Module Mapping

### `crates/codex-runtime/src/adapters/web/tests`

- `serialization`: unit
- `approval_boundaries`: contract
- `contract_and_spawn`: contract
- `approvals`: integration
- `routing_observability`: integration
- `session_flows`: integration

### `crates/codex-runtime/src/appserver/tests`

- `contract`: unit
- `validated_calls`: contract
- `server_requests`: integration

### `crates/codex-runtime/src/runtime/api/tests`

- `params_and_types`: unit
- `thread_api`: contract plus integration
- `run_prompt`: integration
- `command_exec`: contract plus integration

### `crates/codex-runtime/src/domain/artifact/tests`

- `unit_core`: unit
- `collect_output`: contract
- `runtime_tasks`: integration

### `crates/codex-runtime/src/ergonomic/tests`

- `unit`: unit
- `real_server`: opt-in integration only

### `crates/codex-runtime/src/plugin/tests`

- `hook_report`: unit
- `contract_version`: contract
- `hook_matcher`: unit

### `crates/codex-runtime/src/runtime/core/tests.rs`

- core lifecycle and runtime wiring integration coverage

## De-duplication Rules

- do not prove the same invariant in every layer
- if a pure helper or validator is fully covered in `unit`, do not re-test that logic through large integration paths without a new interaction risk
- integration tests should focus on lifecycle, state, concurrency, and boundary I/O
- new typed RPC parity work should usually land in `unit + contract + mock integration`

## Release Gates

Default verification:

```bash
cargo test --workspace
```

Opt-in real-server verification:

```bash
CODEX_RUNTIME_REAL_SERVER_APPROVED=1 \
cargo test -p codex-runtime ergonomic::tests::real_server:: -- --ignored --nocapture
```

Focused examples:

```bash
cargo test -p codex-runtime runtime::api::tests::params_and_types:: -- --nocapture
cargo test -p codex-runtime adapters::web::tests::contract_and_spawn:: -- --nocapture
cargo test -p codex-runtime domain::artifact::tests::runtime_tasks:: -- --nocapture
```
