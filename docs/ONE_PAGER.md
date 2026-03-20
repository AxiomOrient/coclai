# Codex Runtime One-Pager

## What It Is

`codex-runtime` is a Rust crate that wraps the local `codex app-server` and exposes progressively lower-level integration layers.

It lets consumers choose between:
- one-shot prompt execution
- reusable workflows
- explicit session lifecycle
- recurring automation
- validated app-server RPC helpers
- raw runtime control

## Why It Exists

The upstream `codex` CLI already owns the app-server process and protocol. This crate makes that substrate easier to consume from Rust without forcing every caller to hand-roll:
- stdio process management
- JSON-RPC request and response typing
- session and thread lifecycle
- approval and hook wiring
- streaming event filtering
- web and artifact higher-order adapters

## Layer Model

| Layer | Entry point | Main value |
|-------|-------------|------------|
| 1 | `quick_run` | smallest possible path |
| 2 | `Workflow` | repeated runs with simple config |
| 3 | `Client` + `Session` | canonical typed integration surface |
| 4 | `automation::spawn` | recurring work on one prepared session |
| 5 | `AppServer` | low-level validated JSON-RPC |
| 6 | `Runtime` | full control, live streams, raw mode |

## Core Design

- one repository, one crate
- high-level APIs stay intentionally smaller than the full protocol
- stable upstream fields graduate into typed APIs first
- experimental fields stay available through raw JSON-RPC
- validation is strict by default
- side effects are pushed to the boundary when practical; internal helpers prefer data-first decisions and pure transforms

## Main Modules

- `runtime`: sessions, runtime lifecycle, typed models, approvals, hooks, metrics
- `plugin`: hook traits and compatibility contracts
- `automation`: recurring prompt runner over one session
- `web`: tenant/session-oriented web bridge above runtime sessions
- `artifact`: artifact/task domain above runtime threads and stores

## Operational Defaults

- approval: `never`
- sandbox: `read-only`
- effort: `medium`
- timeout: `120s`
- privileged escalation: off unless explicitly enabled

## Important Contracts

- `Session::ask_stream(...)` gives scoped turn streaming; `finish()` yields final turn result
- dropping an unfinished prompt stream triggers best-effort interrupt and cleanup
- automation is session-scoped, single-flight, and non-durable
- privileged sandbox usage is validated on typed thread-start and turn-start paths
- typed RPC validation is stricter than raw mode by design

## Test Strategy

- `unit`: pure transforms and data validation
- `contract`: protocol and security boundaries
- `integration`: lifecycle and module wiring
- real-server tests are opt-in, not part of the default deterministic gate

## Who Should Use Which Surface

- library users who want the fastest start: `quick_run` or `Workflow`
- backend/service integrations: `runtime::{Client, Session}`
- low-level parity consumers: `AppServer`
- framework or platform builders: `Runtime`, `web`, `artifact`

## Current Version Snapshot

As of `0.6.1`, the project includes:
- typed scoped session streaming
- `ClientConfig` control for app-server launch environment, cwd, and extra args
- built-in `web` and `artifact` modules
- stricter typed RPC validation and clearer cleanup/metrics boundaries
