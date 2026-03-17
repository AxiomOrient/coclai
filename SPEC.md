# SPEC

Status: active repository-level contract for the `0.5.x` line

## 1. Product Identity

- repository: `codex-runtime`
- published crate: `codex-runtime`
- Rust import path: `codex_runtime`
- role: reusable Rust substrate around the local `codex app-server`

## 2. Repository Objective

`codex-runtime` exists to provide a **typed, safe, reusable local integration surface**
for `codex` app-server sessions and requests.

It is intentionally positioned as a **substrate**:

- it owns transport, session lifecycle, validation, typed app-server access, hooks,
  and release gates
- it does **not** own worker semantics such as `goal`, `run`, `status`, `resume`,
  `abort`, or product-level outcome vocabularies
- it should be directly reusable by consumers such as `AxiomRunner` without semantic
  renaming

## 3. Owned Scope

This repository owns the following shipped surfaces:

- child-process startup/shutdown and stdio transport for the local `codex` binary
- runtime/session/client lifecycle and typed request/response models
- validated `AppServer` helpers and controlled raw JSON-RPC access
- approval routing and server-request handling
- plugin / hook contracts
- built-in web bridge surface
- built-in artifact-domain surface
- release gates, compatibility guards, and test boundaries

## 4. Non-Goals

This repository does not own:

- worker semantics (`goal`, `run`, `status`, `resume`, `abort`, `replay`, `doctor`)
- control-plane semantics or operator read models
- generic methodology or package-standard content
- semantic aliases over Codex or `codex-runtime` values
- durable orchestration inside `automation`
- a multi-crate packaging split in v1

`automation` remains intentionally small and session-scoped. It is not a durable job
scheduler or orchestration engine.

## 5. Packaging Model

The package model is intentionally simple:

- **one repository**
- **one published crate**
- **one canonical repository spec** (`SPEC.md`)

All public modules currently ship from the default crate surface. The repository may
classify modules by **tier**, but not by separate package ownership in v1.

### 5.1 Built-in by default

The following surfaces ship together in the default crate:

- substrate core (`runtime`, `AppServer`, `plugin`, `rpc_methods`)
- built-in higher-order modules (`web`, `artifact`)
- convenience layers (`quick_run`, `Workflow`, `automation`)

### 5.2 Not chosen in v1

The following are intentionally **not** part of the v1 package model:

- Cargo feature-gated module split
- multiple published crates for web/artifact/core
- a parallel `specs/` tree that duplicates this repository-level contract

If the packaging model changes materially in the future, `SPEC.md`, `README.md`, and
`docs/API_REFERENCE.md` must be updated in the same change.

## 6. Surface Tiers

### 6.1 Foundation Core

The canonical substrate core is:

- `codex_runtime::runtime`
- `codex_runtime::AppServer`
- `codex_runtime::rpc_methods`
- `codex_runtime::plugin`

This tier owns typed lifecycle, transport, validation, requests, approvals, and hook
contracts.

### 6.2 Built-in Higher-Order Modules

The following modules ship in the default crate and are part of the supported public
surface:

- `codex_runtime::web`
- `codex_runtime::artifact`

They are built-in modules, not separate packages.

### 6.3 Convenience Layers

The following surfaces remain convenience layers above the substrate core:

- `quick_run`
- `quick_run_with_profile`
- `Workflow`, `WorkflowConfig`
- `automation::{spawn, AutomationSpec, ...}`

These layers are useful for local consumers, examples, and lightweight workflows, but
they are not the canonical bridge for products that already own their own run
semantics.

### 6.4 Raw / Escape-Hatch Access

Raw JSON-RPC remains available through the runtime/app-server lower layers when typed
coverage is not yet available. Raw mode exists for parity and experimentation, not as
the default integration profile.

## 7. Safety And Validation Invariants

The repository contract keeps the following invariants:

- safe defaults stay aligned across entry points:
  - approval: `never`
  - sandbox: `read-only`
  - effort: `medium`
  - timeout: `120s`
  - privileged escalation: `false` unless explicitly opted in
- typed paths stay validated by default; raw paths are explicit escape hatches
- plugin compatibility is major-version gated through `PluginContractVersion`
- live, real-server verification remains opt-in and outside the deterministic default
  release boundary
- release quality is enforced by the shipped check scripts and preflight flow

## 8. Canonical AxiomRunner Consumer Profile

When `AxiomRunner` integrates with `codex-runtime`, the canonical dependency surface is
the **core substrate API**, not the convenience or extension layers.

### 8.1 Preferred surface

`AxiomRunner` should prefer:

- `runtime::{Client, Session, ClientConfig, SessionConfig, RunProfile}`
- `runtime::{PromptRunParams, PromptRunResult, PromptRunError}`
- `runtime::{ServerRequest, ServerRequestConfig}`
- other typed runtime models needed for explicit session and approval handling

### 8.2 Allowed low-level parity surface

`AxiomRunner` may use the following when validated low-level parity is required:

- `AppServer`
- `rpc_methods`

### 8.3 Not canonical for AxiomRunner

The following must not become the primary bridge surface for `AxiomRunner`:

- `quick_run`
- `Workflow`
- `automation`
- `web`
- `artifact`

Those surfaces may still exist for other consumers, tests, examples, or future
specialized integrations, but they are not the canonical worker bridge.

## 9. Documentation Structure

The documentation stack is:

- `README.md` — quick orientation and entrypoint map
- `SPEC.md` — canonical repository-level truth
- `docs/API_REFERENCE.md` — detailed stable API surface and payload contracts
- `docs/TEST_TREE.md` — test-layer structure and live verification boundary
- `docs/BACKLOG.md` — non-blocking follow-up work
- temporary planning artifacts may exist during an active delivery cycle, but they are
  not part of the durable release documentation surface and do not override `SPEC.md`
  or shipped code

## 10. Cleanup And Drift Rules

- migration-only documents should be removed once their durable truth is absorbed into
  `SPEC.md`, `README.md`, and active reference docs
- packaging truth must not be duplicated across multiple conflicting documents
- if a module is shipped by default in code, documentation must not describe it as
  optional
- if `AxiomRunner` changes its canonical bridge profile, update this file and the API
  reference in the same delivery cycle
