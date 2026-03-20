# Codex Runtime

`codex-runtime` is a Rust wrapper around the local `codex app-server`, the stdio JSON-RPC backend started by the `codex` CLI.

Repository identity:
- repository and crate: `codex-runtime`
- Rust import path: `codex_runtime`
- current crate version: `0.6.1`

The project is intentionally layered so callers can start with one prompt and move down only when they need more control.

| Layer | Entry point | Use when |
|-------|-------------|----------|
| 1 | `quick_run`, `quick_run_with_profile` | You want one prompt with safe defaults |
| 2 | `Workflow`, `WorkflowConfig` | You want repeated runs in one working directory |
| 3 | `runtime::{Client, Session}` | You want explicit session lifecycle and typed config |
| 4 | `automation::{spawn, AutomationSpec}` | You want repeated turns on one prepared `Session` |
| 5 | `AppServer` | You want validated low-level JSON-RPC helpers |
| 6 | `runtime::Runtime` or raw JSON-RPC | You want full runtime control and live events |

## Install

Requires `codex` CLI `>= 0.104.0` on `$PATH`.

Published crate:

```toml
[dependencies]
codex-runtime = "0.6.1"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

Workspace path:

```toml
[dependencies]
codex-runtime = { path = "crates/codex-runtime" }
```

## Safe Defaults

All high-level entry points share the same baseline unless you opt out:

| Setting | Default |
|---------|---------|
| approval | `never` |
| sandbox | `read-only` |
| effort | `medium` |
| timeout | `120s` |
| privileged escalation | `false` |

Privileged execution must be enabled explicitly. Tool-use hooks do not bypass sandbox or approval policy.

## Quick Start

### One-shot prompt

```rust
use codex_runtime::quick_run;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = quick_run("/abs/path/workdir", "Summarize this repo in 3 bullets").await?;
    println!("{}", out.assistant_text);
    Ok(())
}
```

### Reusable workflow

```rust
use codex_runtime::{Workflow, WorkflowConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let workflow = Workflow::connect(
        WorkflowConfig::new("/abs/path/workdir")
            .with_model("gpt-4o")
            .attach_path("docs/API_REFERENCE.md"),
    )
    .await?;

    let out = workflow.run("Summarize only the public API").await?;
    println!("{}", out.assistant_text);

    workflow.shutdown().await?;
    Ok(())
}
```

### Explicit client and session

```rust
use codex_runtime::runtime::{Client, SessionConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::connect_default().await?;
    let session = client
        .start_session(SessionConfig::new("/abs/path/workdir"))
        .await?;

    let first = session.ask("Summarize the current design").await?;
    let second = session.ask("Reduce that to 3 lines").await?;

    println!("{}", first.assistant_text);
    println!("{}", second.assistant_text);

    session.close().await?;
    client.shutdown().await?;
    Ok(())
}
```

### Scoped streaming

```rust
use codex_runtime::runtime::{Client, PromptRunStreamEvent, SessionConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::connect_default().await?;
    let session = client
        .start_session(SessionConfig::new("/abs/path/workdir"))
        .await?;

    let mut stream = session.ask_stream("Explain the current module boundaries").await?;

    while let Some(event) = stream.recv().await? {
        if let PromptRunStreamEvent::AssistantMessageDelta(delta) = event {
            print!("{delta}");
        }
    }

    let final_result = stream.finish().await?;
    println!("\nturn={} text={}", final_result.turn_id, final_result.assistant_text);

    session.close().await?;
    client.shutdown().await?;
    Ok(())
}
```

`Session::ask_wait(prompt)` is the convenience path for `ask_stream(...).finish().await` when you do not need manual event handling.

### Automation

```rust
use std::time::{Duration, SystemTime};

use codex_runtime::automation::{spawn, AutomationSpec};
use codex_runtime::runtime::{Client, SessionConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::connect_default().await?;
    let session = client
        .start_session(SessionConfig::new("/abs/path/workdir"))
        .await?;

    let handle = spawn(
        session,
        AutomationSpec {
            prompt: "Keep reducing the backlog one item at a time".to_owned(),
            start_at: Some(SystemTime::now() + Duration::from_secs(60)),
            every: Duration::from_secs(1800),
            stop_at: Some(SystemTime::now() + Duration::from_secs(8 * 3600)),
            max_runs: None,
        },
    );

    let status = handle.wait().await;
    println!("{status:?}");
    client.shutdown().await?;
    Ok(())
}
```

Automation contract:
- one prepared `Session` per runner
- fixed `Duration` cadence only
- one turn in flight at a time
- missed ticks collapse into one next eligible run
- any `PromptRunError` is terminal
- no cron parsing, persistence, or restart recovery in v1

## Public Modules

| Module | Role |
|--------|------|
| `codex_runtime` | root convenience surface |
| `codex_runtime::runtime` | typed runtime, sessions, approvals, transport, hooks, metrics |
| `codex_runtime::automation` | session-scoped recurring prompt runner |
| `codex_runtime::plugin` | hook traits and hook-side contracts |
| `codex_runtime::web` | higher-order web bridge over runtime sessions and approvals |
| `codex_runtime::artifact` | higher-order artifact domain over runtime threads and stores |

Root crate exports include:
- `quick_run`, `quick_run_with_profile`, `QuickRunError`
- `Workflow`, `WorkflowConfig`
- `AppServer`, `rpc_methods`
- `HookMatcher`, `FilteredPreHook`, `FilteredPostHook`, `ShellCommandHook`
- `automation`, `plugin`, `runtime`, `web`, `artifact`

## Runtime Contracts

- High-level builders stay intentionally smaller than raw upstream payloads.
- Stable upstream fields graduate into typed APIs first.
- Experimental or custom methods remain available through raw JSON-RPC.
- Validation is strict in typed paths and only relaxed when callers explicitly choose raw mode.
- Detached cleanup and validation paths are kept data-first where practical so side effects stay at the outer boundary.

## Hooks

Hooks let you intercept lifecycle phases without forking the runtime call path.

Phases:
- `PreRun`, `PostRun`
- `PreSessionStart`, `PostSessionStart`
- `PreTurn`, `PostTurn`
- `PreToolUse`, `PostToolUse`

Key rules:
- pre-hooks can mutate or block
- post-hooks observe outcomes and issue reports
- tool-use hooks run inside approval-gated command/file-change handling
- hook logic sits on top of sandbox and approval policy, not instead of it

## Documentation

- [`docs/ONE_PAGER.md`](docs/ONE_PAGER.md): one-page project summary
- [`docs/API_REFERENCE.md`](docs/API_REFERENCE.md): public API and contract reference
- [`docs/TEST_TREE.md`](docs/TEST_TREE.md): test layers and release-gate boundaries
- [`docs/README.md`](docs/README.md): documentation index
- [`CHANGELOG.md`](CHANGELOG.md): release history

## Quality Gates

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Opt-in real-server tests:

```bash
CODEX_RUNTIME_REAL_SERVER_APPROVED=1 \
cargo test -p codex-runtime ergonomic::tests::real_server:: -- --ignored --nocapture
```

## License

MIT
