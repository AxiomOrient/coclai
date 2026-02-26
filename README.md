# coclai

`coclai`는 로컬 `codex app-server`를 Rust에서 쉽게 다루기 위한 라이브러리 워크스페이스입니다.  
핵심 목표는 "Codex 라이프사이클 단순화 + pre/post hook 기반 워크플로우 조립 + 안정적인 계약/검증 체계"입니다.

## 개요

- 권장 진입점: `crates/coclai` (public facade)
- 기본 사용 흐름: `connect -> run/setup -> ask -> close/shutdown`
- 설계 원칙: 데이터 우선, side effect 분리, fail-open hook 운영

## 핵심 기능

- 고수준 API: `Client`, `Session`
- 초보자용 원샷 API: `quick_run(...)`, `quick_run_with_profile(...)`
- 전문가용 명시적 워크플로우 모델: `WorkflowConfig`, `Workflow`
- JSON-RPC 직결 파사드: `AppServer` + `rpc_methods::*`
- 라이프사이클 API: `run`, `run_with_profile`, `setup`, `ask`, `shutdown`
- pre/post hook 체인
- pre hook 입력 변형 지원 (`prompt`, `model`, `attachments`, `metadata_delta`)
- hook 실패 fail-open (메인 AI 경로는 계속 진행, 오류는 `HookReport`에 누적)
- cross-crate plugin contract 버전 호환성 가드
- 계약/성능/preflight 점검 스크립트 제공

## 워크스페이스 구조

- `crates/coclai`: 공개 API 파사드 (권장)
- `crates/coclai_runtime`: 런타임/RPC/이벤트/승인 처리
- `crates/coclai_plugin_core`: hook/plugin 계약 타입
- `crates/coclai_artifact`: artifact 도메인 어댑터
- `crates/coclai_web`: 웹 세션/이벤트 어댑터

## 요구 사항

- Rust toolchain (edition 2021)
- `codex` CLI 설치 + 로그인
- 활성 스키마 디렉터리
  - `SCHEMAS/app-server/active/metadata.json`
  - `SCHEMAS/app-server/active/manifest.sha256`
  - `SCHEMAS/app-server/active/json-schema/`

스키마 경로 해석 우선순위:

1. `ClientConfig::with_schema_dir(...)`
2. `APP_SERVER_SCHEMA_DIR`
3. 현재 작업 디렉터리의 `SCHEMAS/app-server/active`
4. `crates/coclai_runtime/../../SCHEMAS/app-server/active`

## 설치

현재는 워크스페이스 경로 의존성 기준으로 사용하는 방식이 가장 단순합니다.

```toml
[dependencies]
coclai = { path = "crates/coclai" }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

## 빠른 시작

### 1) 초보자: 원샷 실행 (`quick_run`)

```rust
use coclai::quick_run;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = quick_run("/ABS/PATH/WORKDIR", "요약해줘").await?;
    println!("{}", out.assistant_text);
    Ok(())
}
```

### 2) 단발 실행 (`Client`)

```rust
use coclai::Client;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::connect_default().await?;
    let out = client.run("/ABS/PATH/WORKDIR", "요약해줘").await?;
    println!("{}", out.assistant_text);
    client.shutdown().await?;
    Ok(())
}
```

### 3) 세션 실행

```rust
use coclai::Client;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::connect_default().await?;
    let session = client.setup("/ABS/PATH/WORKDIR").await?;

    let first = session.ask("첫 질문").await?;
    let second = session.ask("두 번째 질문").await?;
    println!("1: {}", first.assistant_text);
    println!("2: {}", second.assistant_text);

    session.close().await?;
    client.shutdown().await?;
    Ok(())
}
```

## 실행 예제

`crates/coclai/examples`에 초보자/전문가 예제를 제공합니다.

```bash
# 초보자 원샷
COCLAI_CWD=/ABS/PATH/WORKDIR COCLAI_PROMPT="요약해줘" \
  cargo run -p coclai --example quick_run

# 전문가 워크플로우
COCLAI_CWD=/ABS/PATH/WORKDIR COCLAI_PROMPT="README 핵심 정리" \
  cargo run -p coclai --example workflow

# JSON-RPC 직접 경로
COCLAI_CWD=/ABS/PATH/WORKDIR COCLAI_PROMPT="hello" \
  cargo run -p coclai --example rpc_direct
```

## 전문가 모드: `WorkflowConfig` + `Workflow`

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
    let out = workflow.run("이 파일 핵심 요약").await?;
    println!("{}", out.assistant_text);
    workflow.shutdown().await?;
    Ok(())
}
```

## JSON-RPC 직접 경로: `AppServer`

`AppServer`는 codex app-server JSON-RPC를 직접 다루는 얇은 파사드입니다.

- `request_json(...)`: known method 기준 params/result 정합성 검증 포함
- `request_json_unchecked(...)`: 실험/커스텀 메서드 호출
- `notify_json(...)`: known method 기준 params 정합성 검증 포함
- `take_server_requests(...)` + `respond_server_request_*`: 승인/입력 요청 루프 처리

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

## 프로파일 직접 사용 (모델/정책/첨부)

```rust
use coclai::{ApprovalPolicy, Client, RunProfile, SandboxPolicy, SandboxPreset};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = Client::connect_default().await?;

    let profile = RunProfile::new()
        .with_model("gpt-5-codex")
        .with_approval_policy(ApprovalPolicy::OnRequest)
        .with_sandbox_policy(SandboxPolicy::Preset(SandboxPreset::WorkspaceWrite {
            writable_roots: vec!["/ABS/PATH/WORKDIR".to_owned()],
            network_access: false,
        }))
        .attach_path("README.md");

    let out = client
        .run_with_profile("/ABS/PATH/WORKDIR", "이 파일 핵심 요약", profile)
        .await?;

    println!("{}", out.assistant_text);
    client.shutdown().await?;
    Ok(())
}
```

## Hook 워크플로우

- Hook 등록 API
  - `ClientConfig::with_pre_hook(...)`, `ClientConfig::with_post_hook(...)`
  - `RunProfile::with_pre_hook(...)`, `RunProfile::with_post_hook(...)`
  - `SessionConfig::with_pre_hook(...)`, `SessionConfig::with_post_hook(...)`
- 실행 의미
  - `pre_*`는 메인 작업 전 준비/입력 정규화
  - `post_*`는 메인 작업 후 정리/기록
  - hook 오류는 fail-open으로 처리되고 `HookReport`로 수집

## 주요 API

- `quick_run(...)`
- `quick_run_with_profile(...)`
- `WorkflowConfig::new(...)`
- `Workflow::connect(...)`
- `Workflow::run(...)`
- `Workflow::setup_session(...)`
- `Workflow::shutdown(...)`
- `AppServer::connect_default(...)`
- `AppServer::request_json(...)`
- `AppServer::request_json_unchecked(...)`
- `AppServer::notify_json(...)`
- `AppServer::take_server_requests(...)`
- `AppServer::respond_server_request_ok(...)`
- `AppServer::respond_server_request_err(...)`
- `rpc_methods::*`
- `Client::run(...)`
- `Client::run_with(...)`
- `Client::run_with_profile(...)`
- `Client::setup(...)`
- `Client::setup_with_profile(...)`
- `Client::resume_session(...)`
- `Session::ask(...)`
- `Session::ask_with(...)`
- `Session::ask_with_profile(...)`
- `Session::interrupt_turn(...)`
- `Session::close(...)`
- `Client::shutdown(...)`

가장 단순한 JSON-RPC 직접 사용은 `AppServer`, 더 로우레벨 제어는 `coclai::runtime` (`coclai_runtime`)를 사용하세요.

## 로컬 검증

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
./scripts/release_preflight.sh
```

## 문서

- 아키텍처: `Docs/ARCHITECTURE.md`
- 공개 API: `Docs/CORE_API.md`
- 스키마/계약: `Docs/SCHEMA_AND_CONTRACT.md`
- 보안: `Docs/SECURITY.md`

## 라이선스

MIT
