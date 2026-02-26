# coclai

`coclai`는 로컬 `codex app-server`를 Rust에서 안정적으로 감싸는(workspace wrapper) 라이브러리입니다.

핵심 목표:
- Codex 라이프사이클 단순화 (`connect -> run/setup -> ask -> close/shutdown`)
- `pre/post` hook 기반 워크플로우 조립
- JSON-RPC 계약 정합성 검증과 릴리즈 게이트 자동화

## Table of Contents
- [What This Project Solves](#what-this-project-solves)
- [Workspace Layout](#workspace-layout)
- [Requirements](#requirements)
- [Installation](#installation)
- [Quick Start](#quick-start)
- [Usage Paths](#usage-paths)
- [Hooks (pre/post)](#hooks-prepost)
- [JSON-RPC Direct Usage](#json-rpc-direct-usage)
- [API Cheat Sheet](#api-cheat-sheet)
- [Validation and Release Gates](#validation-and-release-gates)
- [Troubleshooting](#troubleshooting)
- [Project Docs](#project-docs)
- [License](#license)

## What This Project Solves

Codex를 Rust에서 직접 붙일 때 반복되는 문제를 줄입니다.
- 프로세스 스폰/종료, 세션 수명주기, 스키마 검사, 에러 매핑을 표준화
- 초보자 경로(`quick_run`)와 전문가 경로(`WorkflowConfig`, `AppServer`)를 동시에 제공
- hook 실패 시 메인 경로를 중단하지 않는 fail-open 정책으로 운영 안정성 확보

## Workspace Layout

- `crates/coclai`: 공개 파사드 (권장 진입점)
- `crates/coclai_runtime`: 런타임/JSON-RPC/상태/승인 처리
- `crates/coclai_plugin_core`: hook/plugin 공통 계약 타입
- `crates/coclai_artifact`: artifact 도메인 어댑터
- `crates/coclai_web`: web 세션/이벤트 어댑터
- `SCHEMAS/`: app-server 활성 스키마 + 골든 이벤트
- `scripts/`: 검증/스키마/릴리즈 보조 스크립트

## Requirements

- Rust 2021 toolchain
- `codex` CLI 설치 및 로그인 상태
- 활성 스키마 디렉터리 존재:
  - `SCHEMAS/app-server/active/metadata.json`
  - `SCHEMAS/app-server/active/manifest.sha256`
  - `SCHEMAS/app-server/active/json-schema/`

스키마 경로 해석 우선순위:
1. `ClientConfig::with_schema_dir(...)`
2. 환경변수 `APP_SERVER_SCHEMA_DIR`
3. 현재 작업 디렉터리의 `SCHEMAS/app-server/active`
4. 패키지 기본 경로 `crates/coclai_runtime/../../SCHEMAS/app-server/active`

런타임 호환성 기본 가드:
- `initialize.userAgent` 존재 요구
- Codex 런타임 최소 버전: `0.104.0`

## Installation

현재 저장소 기준 사용이 기본 경로입니다.

```toml
[dependencies]
coclai = { path = "crates/coclai" }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

## Quick Start

### 1) One-shot (가장 쉬운 경로)

```rust
use coclai::quick_run;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = quick_run("/ABS/PATH/WORKDIR", "이 디렉터리 핵심을 3줄로 요약해줘").await?;
    println!("{}", out.assistant_text);
    Ok(())
}
```

### 2) Client + Session (명시적 라이프사이클)

```rust
use coclai::Client;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::connect_default().await?;
    let session = client.setup("/ABS/PATH/WORKDIR").await?;

    let a = session.ask("첫 질문").await?;
    let b = session.ask("두 번째 질문").await?;
    println!("A: {}", a.assistant_text);
    println!("B: {}", b.assistant_text);

    session.close().await?;
    client.shutdown().await?;
    Ok(())
}
```

### 3) 실행 가능한 예제

```bash
# one-shot
COCLAI_CWD=/ABS/PATH/WORKDIR COCLAI_PROMPT="요약해줘" \
  cargo run -p coclai --example quick_run

# workflow (expert defaults)
COCLAI_CWD=/ABS/PATH/WORKDIR COCLAI_PROMPT="README 핵심 정리" \
  cargo run -p coclai --example workflow

# JSON-RPC direct
COCLAI_CWD=/ABS/PATH/WORKDIR COCLAI_PROMPT="hello" \
  cargo run -p coclai --example rpc_direct
```

## Usage Paths

### Beginner Path
- `quick_run(cwd, prompt)`
- `quick_run_with_profile(cwd, prompt, profile)`

특징:
- 연결/실행/종료를 한 번에 수행
- 최소 코드로 빠르게 시작 가능

### Expert Path
- `WorkflowConfig`로 데이터 모델을 먼저 고정
- `Workflow`로 재사용 가능한 실행 핸들 운영

```rust
use coclai::{
    ApprovalPolicy, ReasoningEffort, SandboxPolicy, SandboxPreset, Workflow, WorkflowConfig,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cfg = WorkflowConfig::new("/ABS/PATH/WORKDIR")
        .with_model("gpt-5-codex")
        .with_effort(ReasoningEffort::High)
        .with_approval_policy(ApprovalPolicy::OnRequest)
        .with_sandbox_policy(SandboxPolicy::Preset(SandboxPreset::WorkspaceWrite {
            writable_roots: vec!["/ABS/PATH/WORKDIR".to_owned()],
            network_access: false,
        }))
        .attach_path("README.md");

    let workflow = Workflow::connect(cfg).await?;
    let out = workflow.run("핵심만 정리해줘").await?;
    println!("{}", out.assistant_text);
    workflow.shutdown().await?;
    Ok(())
}
```

## Hooks (pre/post)

등록 가능한 위치:
- `ClientConfig::with_pre_hook(...)`, `ClientConfig::with_post_hook(...)`
- `RunProfile::with_pre_hook(...)`, `RunProfile::with_post_hook(...)`
- `SessionConfig::with_pre_hook(...)`, `SessionConfig::with_post_hook(...)`

실행 의미:
- `pre_*`: 메인 호출 전 준비/정규화
- `post_*`: 메인 호출 후 정리/기록

실행 정책:
- Hook 체인 순서: `pre -> core call -> post`
- pre hook 입력 변형 허용 필드: `prompt`, `model`, `attachments`, `metadata_delta`
- Hook 오류 처리: fail-open
  - 메인 AI 실행은 계속 진행
  - 오류는 `HookReport`에 누적

공통 계약 타입:
- `HookPhase`, `HookContext`, `HookPatch`, `HookAction`, `HookIssue`, `HookReport`
- `PluginContractVersion` (major 기준 호환성 체크)

## JSON-RPC Direct Usage

`AppServer`는 codex app-server JSON-RPC를 직접 다루는 얇은 파사드입니다.

- 검증 호출: `request_json`, `notify_json`
- 검증 모드 지정: `request_json_with_mode`, `notify_json_with_mode`
- 비검증 호출: `request_json_unchecked`, `notify_json_unchecked`
- 서버 요청 루프: `take_server_requests`
- 승인 응답: `respond_server_request_ok`, `respond_server_request_err`

제공 상수 (`rpc_methods`):
- `thread/start`, `thread/resume`, `thread/fork`, `thread/archive`
- `thread/read`, `thread/list`, `thread/loaded/list`, `thread/rollback`
- `turn/start`, `turn/interrupt`

```rust
use coclai::{rpc_methods, AppServer};
use serde_json::json;

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let app = AppServer::connect_default().await?;
    let thread = app.request_json(rpc_methods::THREAD_START, json!({})).await?;
    println!("{thread}");
    app.shutdown().await?;
    Ok(())
}
```

## API Cheat Sheet

고수준:
- `Client::connect_default`, `Client::connect`
- `Client::run`, `Client::run_with`, `Client::run_with_profile`
- `Client::setup`, `Client::setup_with_profile`, `Client::start_session`, `Client::resume_session`
- `Client::continue_session`, `Client::continue_session_with`, `Client::continue_session_with_profile`
- `Client::interrupt_session_turn`, `Client::close_session`, `Client::shutdown`
- `Session::ask`, `Session::ask_with`, `Session::ask_with_profile`, `Session::interrupt_turn`, `Session::close`

워크플로우:
- `WorkflowConfig::new`
- `Workflow::connect`, `Workflow::run`, `Workflow::run_with_profile`
- `Workflow::setup_session`, `Workflow::setup_session_with_profile`, `Workflow::shutdown`

원샷:
- `quick_run`, `quick_run_with_profile`

직접 RPC:
- `AppServer` + `rpc_methods::*`

## Validation and Release Gates

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
./scripts/release_preflight.sh
```

스키마 관련:
```bash
./scripts/update_schema.sh
./scripts/check_schema_manifest.sh
```

성능 회귀 체크:
```bash
./scripts/run_micro_bench.sh
```

## Troubleshooting

### `SchemaDirNotFound` / `SchemaDirNotDirectory`
- `SCHEMAS/app-server/active` 경로 구조 확인
- 필요하면 `ClientConfig::with_schema_dir(...)`로 명시 지정

### `MissingInitializeUserAgent` / `IncompatibleCodexVersion`
- `codex --version` 확인
- 기본 호환성 가드는 Codex `>= 0.104.0` 기준
- 필요 시 `without_compatibility_guard()`로 비활성화 가능

### 세션 종료 후 호출 에러
- `Session::close()` 이후 `ask/interrupt_turn`은 로컬에서 즉시 거절됨 (의도된 동작)

## Project Docs

- Architecture: `Docs/ARCHITECTURE.md`
- Core API: `Docs/CORE_API.md`
- Schema & Contract: `Docs/SCHEMA_AND_CONTRACT.md`
- Security: `Docs/SECURITY.md`

## License

MIT
