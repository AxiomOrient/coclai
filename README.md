# coclai

`coclai` is a Rust wrapper around the local `codex app-server`—the stdio JSON-RPC backend spawned by the `codex` CLI binary.

It exposes five layers so you can start simple and reach deeper only when needed:

| Layer | Entry point | When to use |
|-------|-------------|-------------|
| 1 | `quick_run`, `quick_run_with_profile` | One prompt, disposable session |
| 2 | `Workflow`, `WorkflowConfig` | Repeated runs in one working directory |
| 3 | `runtime::{Client, Session}` | Explicit session lifecycle, resume, interrupt |
| 4 | `AppServer` | Direct JSON-RPC with typed helpers and server-request loop |
| 5 | `runtime::Runtime` or raw JSON-RPC | Full control, live events, experimental access |

## Install

**Requires:** `codex` CLI >= 0.104.0 installed and available on `$PATH`.

Published crate:
```toml
[dependencies]
coclai = "0.2.1"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

Local workspace dependency:
```toml
[dependencies]
coclai = { path = "crates/coclai" }
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
use coclai::quick_run;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = quick_run("/abs/path/workdir", "Summarize this repo in 3 bullets").await?;
    println!("{}", out.assistant_text);
    Ok(())
}
```

### `Workflow`
```rust
use coclai::{Workflow, WorkflowConfig};

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
use coclai::runtime::{Client, SessionConfig};

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

### `AppServer`
```rust
use coclai::runtime::CommandExecParams;
use coclai::AppServer;
use serde_json::json;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let app = AppServer::connect_default().await?;

    let _thread = app
        .request_json(
            coclai::rpc_methods::THREAD_START,
            json!({
                "cwd": "/abs/path/workdir",
                "sandbox": { "type": "readOnly" }
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
| `coclai` | Root: `quick_run`, `Workflow`, `WorkflowConfig`, `AppServer`, `rpc_methods` |
| `coclai::runtime` | Low-level runtime: `Client`, `Session`, `Runtime`, typed models, errors |
| `coclai::plugin` | Hook extension point: `PreHook`, `PostHook`, `HookContext`, `HookPatch` |
| `coclai::web` | Optional HTTP adapter bridging runtime sessions to SSE/REST web services |
| `coclai::artifact` | Optional artifact tracking domain built on top of the runtime |

Important runtime submodules available for direct use when re-exports are not enough:
`runtime::api`, `runtime::approvals`, `runtime::client`, `runtime::core`,
`runtime::errors`, `runtime::events`, `runtime::hooks`, `runtime::metrics`,
`runtime::rpc`, `runtime::rpc_contract`, `runtime::sink`, `runtime::state`,
`runtime::transport`, `runtime::turn_output`

## Hooks

Hooks let you intercept and mutate prompt calls at defined lifecycle phases without forking the call path.

```rust
use std::sync::Arc;
use coclai::{WorkflowConfig, plugin::{PreHook, HookContext, HookAction, HookFuture, HookIssue}};

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

Hook phases: `PreRun`, `PostRun`, `PreSessionStart`, `PostSessionStart`, `PreTurn`, `PostTurn`.

## Documentation

- [API_REFERENCE.md](docs/API_REFERENCE.md): full public API surface, typed payload contracts, validation and security rules
- [TEST_TREE.md](docs/TEST_TREE.md): test layer structure and live-gate boundary

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
COCLAI_REAL_SERVER_APPROVED=1 \
COCLAI_RELEASE_INCLUDE_REAL_SERVER=1 \
./scripts/release_preflight.sh
```

## Design Boundaries

- High-level APIs stay small on purpose.
- Stable non-experimental upstream fields go to typed APIs first.
- Experimental fields stay raw until the protocol is stable and testable.
- `requestUserInput` and dynamic tool-call live coverage remain outside the deterministic release boundary.

## License

MIT
