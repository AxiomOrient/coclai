# Codex Runtime

`Codex Runtime` is the repository for a Rust wrapper around the local `codex app-server`—the stdio JSON-RPC backend spawned by the `codex` CLI binary.

Current identity:
- repository and package name: `codex-runtime`
- Rust import path: `codex_runtime`

It exposes six layers so you can start simple and reach deeper only when needed:

| Layer | Entry point | When to use |
|-------|-------------|-------------|
| 1 | `quick_run`, `quick_run_with_profile` | One prompt, disposable session |
| 2 | `Workflow`, `WorkflowConfig` | Repeated runs in one working directory |
| 3 | `runtime::{Client, Session}` | Explicit session lifecycle, resume, interrupt |
| 4 | `automation::{spawn, AutomationSpec}` | Schedule repeated turns on one prepared `Session` |
| 5 | `AppServer` | Direct JSON-RPC with typed helpers and server-request loop |
| 6 | `runtime::Runtime` or raw JSON-RPC | Full control, live events, experimental access |

## Install

**Requires:** `codex` CLI >= 0.104.0 installed and available on `$PATH`.

Published crate dependency:
```toml
[dependencies]
codex-runtime = "0.4.0"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

Local workspace dependency:
```toml
[dependencies]
codex-runtime = { path = "crates/codex-runtime" }
```

## Safe Defaults

All entry points share the same safe defaults unless explicitly overridden:

| Setting | Default |
|---------|---------|
| approval | `never` |
| sandbox | `read-only` |
| effort | `medium` |
| timeout | `120s` |
| privileged escalation | `false` (requires explicit opt-in) |

## High-Level API

### `quick_run`
```rust
use codex_runtime::quick_run;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = quick_run("/abs/path/workdir", "Summarize this repo in 3 bullets").await?;
    println!("{}", out.assistant_text);
    Ok(())
}
```

### `Workflow`
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

## Low-Level Typed API

### `Client` and `Session`
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

### `automation::spawn`
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

Contract:
- automation reuses one prepared `Session`; it does not create or resume sessions for you
- scheduling uses absolute `SystemTime` bounds plus one fixed `Duration`
- `every` must be greater than zero
- only one turn is in flight at a time
- missed ticks collapse into one next eligible run
- any `PromptRunError` stops the runner and records `last_error`
- V1 does not provide cron parsing or restart persistence

### `AppServer`
```rust
use codex_runtime::runtime::CommandExecParams;
use codex_runtime::AppServer;
use serde_json::json;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let app = AppServer::connect_default().await?;

    let _thread = app
        .request_json(
            codex_runtime::rpc_methods::THREAD_START,
            json!({
                "cwd": "/abs/path/workdir",
                "sandbox": "read-only"
            }),
        )
        .await?;

    let exec = app
        .command_exec(CommandExecParams {
            command: vec!["pwd".into()],
            cwd: Some("/abs/path/workdir".into()),
            ..CommandExecParams::default()
        })
        .await?;

    println!("{}", exec.stdout);
    app.shutdown().await?;
    Ok(())
}
```

## Defaults And Contracts

- High-level builders stay minimal and do not mirror every upstream field.
- When you need more control, use `RunProfile`, `SessionConfig`, `ClientConfig`, or `RuntimeConfig`.
- Use `AppServer` typed helpers for stable low-level parity.
- Use raw JSON-RPC for experimental or custom methods.

## Public Modules

| Module | Role |
|--------|------|
| `codex_runtime` | Root: `quick_run`, `Workflow`, `WorkflowConfig`, `AppServer`, `rpc_methods`, `HookMatcher`, `FilteredPreHook`, `FilteredPostHook`, `ShellCommandHook` |
| `codex_runtime::automation` | Optional session-scoped recurring prompt runner above one prepared `Session` |
| `codex_runtime::runtime` | Low-level runtime: `Client`, `Session`, `Runtime`, typed models, errors |
| `codex_runtime::plugin` | Hook extension point: `PreHook`, `PostHook`, `HookContext`, `HookPatch` |
| `codex_runtime::web` | Optional HTTP adapter bridging runtime sessions to SSE/REST web services |
| `codex_runtime::artifact` | Optional artifact tracking domain built on top of the runtime |

Important runtime submodules available for direct use when re-exports are not enough:
`runtime::api`, `runtime::approvals`, `runtime::client`, `runtime::core`,
`runtime::errors`, `runtime::events`, `runtime::hooks`, `runtime::metrics`,
`runtime::rpc`, `runtime::rpc_contract`, `runtime::sink`, `runtime::state`,
`runtime::transport`, `runtime::turn_output`

## Optional Modules

### `codex_runtime::web`

Primary entry points:
- `WebAdapter::spawn(runtime, config)` or `spawn_with_adapter(...)`
- `create_session(...)`, `create_turn(...)`, `close_session(...)`
- `subscribe_session_events(...)`, `subscribe_session_approvals(...)`
- `post_approval(...)`
- `new_session_id()`, `serialize_sse_envelope(...)`

Contract:
- one `WebAdapter` bridges runtime threads into tenant/session-scoped web sessions
- approval replies flow back through `post_approval(...)`; callers do not mutate runtime approval state directly

### `codex_runtime::artifact`

Primary entry points:
- `ArtifactSessionManager::new(runtime, store)` or `new_with_adapter(...)`
- `open(artifact_id)` — load or create one artifact-backed runtime thread
- `run_task(spec)` — execute one typed artifact task
- `FsArtifactStore::new(root)` — filesystem-backed store
- pure helpers: `compute_revision(...)`, `validate_doc_patch(...)`, `apply_doc_patch(...)`

Contract:
- the module keeps artifact state in an `ArtifactStore` and delegates runtime turns through an adapter
- compatibility is gated by `PluginContractVersion` before artifact tasks run

## Hooks

Hooks let you intercept and mutate prompt calls at defined lifecycle phases without forking the call path.

```rust
use std::sync::Arc;
use codex_runtime::{WorkflowConfig, plugin::{PreHook, HookContext, HookAction, HookFuture, HookIssue}};

struct LoggingHook;

impl PreHook for LoggingHook {
    fn name(&self) -> &'static str { "logging" }

    fn call<'a>(&'a self, ctx: &'a HookContext) -> HookFuture<'a, Result<HookAction, HookIssue>> {
        Box::pin(async move {
            println!("phase={:?} cwd={:?}", ctx.phase, ctx.cwd);
            Ok(HookAction::Noop)
        })
    }
}

let config = WorkflowConfig::new("/abs/path/workdir")
    .with_global_pre_hook(Arc::new(LoggingHook));
```

Hook phases:
- run/session/turn: `PreRun`, `PostRun`, `PreSessionStart`, `PostSessionStart`, `PreTurn`, `PostTurn`
- tool loop: `PreToolUse`, `PostToolUse`

Hook actions:
- `HookAction::Noop`
- `HookAction::Mutate(HookPatch)`
- `HookAction::Block(BlockReason)` for pre-hooks

Ergonomic builders:
- global hooks: `with_global_pre_hook`, `with_global_post_hook`, `with_global_pre_tool_use_hook`
- run-scoped hooks: `with_run_pre_hook`, `with_run_post_hook`
- shell adapters: `with_shell_pre_hook`, `with_shell_post_hook`, `with_shell_pre_hook_timeout`

Path note:
- `HookMatcher`, `FilteredPreHook`, `FilteredPostHook`, and `ShellCommandHook` are also re-exported at the crate root as `codex_runtime::...`

Important contract:
- pre-tool-use hooks fire on approval-gated tool/file-change requests, not every successful write
- privileged write sandboxes still require explicit opt-in via `allow_privileged_escalation()`
- tool-use hooks do not replace sandbox/approval policy; they sit on top of it

## Documentation

- [docs/README.md](docs/README.md): active documentation index
- [API_REFERENCE.md](docs/API_REFERENCE.md): full public API surface, typed payload contracts, validation and security rules
- [CRATE_RENAME_AUDIT.md](docs/CRATE_RENAME_AUDIT.md): rename verification status and remaining caveats
- [TEST_TREE.md](docs/TEST_TREE.md): test layer structure and live-gate boundary
- [BACKLOG.md](docs/BACKLOG.md): non-blocking follow-up improvements
- [CHANGELOG.md](CHANGELOG.md): release history

## Quality Gates

Deterministic release gates:
```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
./scripts/check_product_hygiene.sh
./scripts/check_security_gate.sh
./scripts/check_blocker_regressions.sh
cargo test --workspace
```

Release preflight:
```bash
./scripts/release_preflight.sh
```

Opt-in real-server preflight:
```bash
CODEX_RUNTIME_REAL_SERVER_APPROVED=1 \
CODEX_RUNTIME_RELEASE_INCLUDE_REAL_SERVER=1 \
./scripts/release_preflight.sh
```

## Design Boundaries

- High-level APIs stay small on purpose.
- Stable non-experimental upstream fields go to typed APIs first.
- Experimental fields stay raw until the protocol is stable and testable.
- `requestUserInput` and dynamic tool-call live coverage remain outside the deterministic release boundary.
- Hook support exists, but live hook coverage is narrower than core prompt/session flows.

## License

MIT
