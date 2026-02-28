# coclai

`coclai`는 로컬 `codex app-server`를 Rust에서 안전하게 감싸는(workspace wrapper) 라이브러리입니다.
핵심은 **라이프사이클 단순화**, **계약 검증 자동화**, **릴리즈 게이트 표준화**입니다.

---

## 목차
- [프로젝트 개요](#프로젝트-개요)
- [무엇이 해결되는가](#무엇이-해결되는가)
- [워크스페이스 구조](#워크스페이스-구조)
- [요구사항 및 호환성](#요구사항-및-호환성)
- [설치](#설치)
- [빠른 시작](#빠른-시작)
- [사용 경로 가이드](#사용-경로-가이드)
- [Hook (pre/post)](#hook-prepost)
- [JSON-RPC 직접 사용](#json-rpc-직접-사용)
- [API 치트시트](#api-치트시트)
- [보안 모델](#보안-모델)
- [스키마/계약 운영](#스키마계약-운영)
- [검증 및 릴리즈 게이트](#검증-및-릴리즈-게이트)
- [외부 공개 릴리즈 체크리스트](#외부-공개-릴리즈-체크리스트)
- [문제 해결](#문제-해결)
- [문서 맵](#문서-맵)
- [기여](#기여)
- [라이선스](#라이선스)

---

## 프로젝트 개요
`coclai`는 다음 3가지를 기본 제공하는 Rust 워크스페이스입니다.

1. Codex 런타임 라이프사이클 표준화 (`connect -> run/setup -> ask -> close -> shutdown`)
2. 스키마/JSON-RPC 계약 검증
3. 릴리즈 전 품질 게이트(포맷/린트/테스트/스키마/문서 동기화)

---

## 무엇이 해결되는가
Codex를 Rust에서 직접 붙일 때 반복되는 운영 문제를 줄입니다.

- 프로세스 spawn/shutdown, 세션 수명주기, 에러 매핑을 API로 고정
- 초보자 경로(`quick_run`)와 전문가 경로(`Workflow`, `AppServer`)를 분리 제공
- Hook 체인(pre/post) 실패 시 메인 실행은 지속(fail-open)
- schema drift/manifest/doc-sync를 게이트로 운영

---

## 워크스페이스 구조

| 경로 | 역할 |
|---|---|
| `crates/coclai` | 공개 파사드(권장 진입점) |
| `crates/coclai_runtime` | 런타임/JSON-RPC/상태/승인 라우팅 |
| `crates/coclai_plugin_core` | hook/plugin 공통 계약 타입 |
| `crates/coclai_artifact` | artifact 도메인 어댑터 |
| `crates/coclai_web` | web 세션/SSE/approval 어댑터 |
| `SCHEMAS/` | app-server active schema + golden fixtures |
| `scripts/` | 검증/스키마/릴리즈 스크립트 |

---

## 요구사항 및 호환성

### 필수 요구사항
- Rust 2021 toolchain
- `codex` CLI 설치 및 로그인 상태
- 활성 스키마 디렉터리:
  - `SCHEMAS/app-server/active/metadata.json`
  - `SCHEMAS/app-server/active/manifest.sha256`
  - `SCHEMAS/app-server/active/json-schema/`

### 스키마 경로 해석 우선순위
1. `ClientConfig::with_schema_dir(...)`
2. `APP_SERVER_SCHEMA_DIR`
3. 현재 작업 디렉터리의 `SCHEMAS/app-server/active`
4. 패키지 기본 경로 (`crates/coclai_runtime/../../SCHEMAS/app-server/active`)

### 호환성 가드
- `initialize.userAgent` 존재 필요
- 최소 Codex 런타임 버전: `0.104.0`

---

## 설치
현재 저장소 기준 사용(로컬 path dependency)이 기본입니다.

```toml
[dependencies]
coclai = { path = "crates/coclai" }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

---

## 빠른 시작

### 1) One-shot (가장 짧은 경로)

```rust
use coclai::quick_run;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = quick_run("/ABS/PATH/WORKDIR", "이 디렉터리 핵심을 3줄로 요약해줘").await?;
    println!("{}", out.assistant_text);
    Ok(())
}
```

### 2) 명시적 세션 경로 (`Client` + `Session`)

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

# workflow (expert safe defaults)
COCLAI_CWD=/ABS/PATH/WORKDIR COCLAI_PROMPT="README 핵심 정리" \
  cargo run -p coclai --example workflow

# workflow (privileged sandbox + explicit opt-in)
COCLAI_CWD=/ABS/PATH/WORKDIR COCLAI_PROMPT="README 핵심 정리" \
  cargo run -p coclai --example workflow_privileged

# JSON-RPC direct (turn/completed까지 기다린 뒤 최종 assistant text 출력)
COCLAI_CWD=/ABS/PATH/WORKDIR COCLAI_PROMPT="hello" \
  cargo run -p coclai --example rpc_direct
```

---

## 사용 경로 가이드

### Beginner Path
- `quick_run(cwd, prompt)`
- `quick_run_with_profile(cwd, prompt, profile)`

특징:
- 연결/실행/종료를 한 번에 수행
- 가장 빠른 온보딩

### Expert Path (Safe Default)

```rust
use coclai::{ReasoningEffort, Workflow, WorkflowConfig};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cfg = WorkflowConfig::new("/ABS/PATH/WORKDIR")
        .with_model("gpt-5-codex")
        .with_effort(ReasoningEffort::High)
        .attach_path("README.md");

    let workflow = Workflow::connect(cfg).await?;
    let out = workflow.run("핵심만 정리해줘").await?;
    println!("{}", out.assistant_text);
    workflow.shutdown().await?;
    Ok(())
}
```

### Expert Path (Privileged Sandbox)
`WorkspaceWrite`/`DangerFullAccess` 계열을 사용할 때는 **반드시** explicit opt-in을 켜야 합니다.

```rust
use coclai::{
    ApprovalPolicy, ReasoningEffort, RunProfile, SandboxPolicy, SandboxPreset, Workflow,
    WorkflowConfig,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cwd = "/ABS/PATH/WORKDIR";
    let profile = RunProfile::new()
        .with_model("gpt-5-codex")
        .with_effort(ReasoningEffort::High)
        .with_approval_policy(ApprovalPolicy::OnRequest)
        .with_sandbox_policy(SandboxPolicy::Preset(SandboxPreset::WorkspaceWrite {
            writable_roots: vec![cwd.to_owned()],
            network_access: false,
        }))
        .allow_privileged_escalation();

    let workflow = Workflow::connect(WorkflowConfig::new(cwd).with_run_profile(profile)).await?;
    let out = workflow.run("핵심만 정리해줘").await?;
    println!("{}", out.assistant_text);
    workflow.shutdown().await?;
    Ok(())
}
```

---

## Hook (pre/post)

등록 지점:
- `ClientConfig::with_pre_hook(...)`, `ClientConfig::with_post_hook(...)`
- `RunProfile::with_pre_hook(...)`, `RunProfile::with_post_hook(...)`
- `SessionConfig::with_pre_hook(...)`, `SessionConfig::with_post_hook(...)`

실행 규칙:
- 순서: `pre -> core call -> post`
- pre hook 입력 변형 허용 필드: `prompt`, `model`, `attachments`, `metadata_delta`
- hook 오류 처리: fail-open
  - 메인 실행은 계속 진행
  - 오류는 `HookReport`에 누적

---

## JSON-RPC 직접 사용
`AppServer`는 codex app-server JSON-RPC를 직접 다루는 얇은 파사드입니다.

제공 호출:
- validated: `request_json`, `notify_json`
- typed: `request_typed`, `notify_typed`
- mode 지정: `request_json_with_mode`, `notify_json_with_mode`, `request_typed_with_mode`, `notify_typed_with_mode`
- unchecked: `request_json_unchecked`, `notify_json_unchecked`
- server request: `take_server_requests`, `respond_server_request_ok`, `respond_server_request_err`

`rpc_methods` 상수:
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

---

## API 치트시트

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

---

## 보안 모델

### 기본값 (Safe-by-default)
- approval 기본값: `never`
- sandbox 기본값: `read-only`
- unknown server request: 기본 `auto_decline_unknown=true`

### 권한 상승(Privileged) 규칙
권한 상승 샌드박스는 아래를 모두 만족해야 허용됩니다.
1. explicit opt-in (`allow_privileged_escalation`) 설정
2. `approval_policy != never`
3. 명시적 실행 범위 (`cwd` 또는 writable roots)

### Web 경계
- tenant/session/thread 교차 접근 금지
- 외부 노출 금지 식별자: 내부 `rpc_id`

---

## 스키마/계약 운영

### 스키마 갱신
```bash
./scripts/update_schema.sh
```

### 드리프트 검사
```bash
./scripts/check_schema_drift.sh
```
- 기본: `COCLAI_SCHEMA_DRIFT_MODE=soft` (warning)
- 차단: `COCLAI_SCHEMA_DRIFT_MODE=hard` (non-zero exit)

### manifest 무결성 검사
```bash
./scripts/check_schema_manifest.sh
```

### 문서-코드 계약 동기화
```bash
./scripts/check_doc_contract_sync.sh
```
- 기본: `COCLAI_DOC_SYNC_MODE=hard`
- strict mismatch 차단: `COCLAI_DOC_SYNC_FAIL_ON_MISMATCH=1` (mismatch verdict 존재 시 non-zero exit)

---

## 검증 및 릴리즈 게이트

### 기본 검증
```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
./scripts/check_product_hygiene.sh
cargo test --workspace
```

### 릴리즈 preflight
```bash
./scripts/release_preflight.sh
```

`release_preflight.sh`는 아래를 포함합니다.
1. `fmt`
2. `clippy`
3. product hygiene
4. 전체 테스트
5. schema drift (**hard-fail**, `source=codex`)
6. schema manifest
7. doc contract sync (**hard + mismatch=0 강제**)

### nightly/opt-in
```bash
./scripts/run_nightly_opt_in_gate.sh
```

### 성능 회귀
```bash
./scripts/run_micro_bench.sh
```

---

## 외부 공개 릴리즈 체크리스트

### Scope / Blast Radius
- 영향 범위: `coclai` 공개 파사드 + 예제 + 릴리즈 게이트 + 문서
- 소비자 영향 반경:
  - `workflow` 예제 사용자
  - `rpc_direct` 예제 사용자
  - 릴리즈 자동화 사용자(`release_preflight.sh`)

### Breaking/Migration 체크
- API 시그니처 변경 없음
- `workflow` 기본 예제는 안전 기본값으로 동작 (마이그레이션 불필요)
- 권한 상승 경로 사용자는 `allow_privileged_escalation` 필요
- 릴리즈 게이트는 drift 발견 시 hard-fail

### Rollout Plan
1. 로컬 게이트 통과 (`fmt`, `clippy`, `test`, `doc-sync`)
2. `release_preflight.sh` 실행
3. drift 해결 후 태그/릴리즈 진행

### Rollback Plan
- 예제 변경 회귀 시: `workflow_privileged` 경로만 유지하고 `workflow`를 이전 상태로 복원
- 릴리즈 게이트 과차단 시: 한시적으로 preflight drift 모드를 soft로 되돌리고 원인 해결 후 재상향

### Stop/Go 기준
- **GO**: preflight 전부 통과 + drift 0 + 문서 동기화 100%
- **STOP**: schema drift hard-fail, doc-sync(coverage/mismatch) 실패, 핵심 테스트 실패

---

## 문제 해결

### `SchemaDirNotFound` / `SchemaDirNotDirectory`
- `SCHEMAS/app-server/active` 구조 확인
- 필요 시 `ClientConfig::with_schema_dir(...)` 사용

### `MissingInitializeUserAgent` / `IncompatibleCodexVersion`
- `codex --version` 확인
- 기본 호환성 가드는 Codex `>= 0.104.0`
- 필요 시 `without_compatibility_guard()` 고려

### `privileged sandbox requires explicit escalation approval`
- privileged sandbox 사용 시 `allow_privileged_escalation()`을 설정했는지 확인
- `approval_policy`가 `never`가 아닌지 확인
- `cwd` 또는 writable roots를 명시했는지 확인

### 세션 종료 후 호출 에러
- `Session::close()` 이후 `ask/interrupt_turn`은 의도적으로 거절됩니다.

---

## 문서 맵
- Architecture: `Docs/ARCHITECTURE.md`
- Core API: `Docs/CORE_API.md`
- Schema & Contract: `Docs/SCHEMA_AND_CONTRACT.md`
- Security: `Docs/SECURITY.md`
- Analysis Contract Matrix: `Docs/analysis/CONTRACT-MATRIX.md`

---

## 기여
- 변경 전 `cargo fmt --check`, `cargo clippy`, `cargo test --workspace` 실행 권장
- 계약/스키마 변경이 있으면 `scripts/*` 게이트까지 함께 확인
- 문서 주장 변경 시 `Docs/analysis/CONTRACT-MATRIX.md`와 동기화 유지

---

## 라이선스
MIT
