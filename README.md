# coclai

`coclai`는 로컬 `codex app-server`를 Rust에서 안전하게 감싸는 **agent-first monolith**입니다.
핵심은 **단일 실행면(coclai-agent)**, **capability ingress parity**, **릴리즈 게이트 표준화**입니다.

---

## 목차
- [프로젝트 개요](#프로젝트-개요)
- [무엇이 해결되는가](#무엇이-해결되는가)
- [워크스페이스 구조](#워크스페이스-구조)
- [요구사항 및 호환성](#요구사항-및-호환성)
- [설치](#설치)
- [빠른 시작](#빠른-시작)
- [사용 경로 가이드](#사용-경로-가이드)
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
`coclai`는 다음 3가지를 기본 제공하는 단일 런타임 crate 기반 프로젝트입니다.

1. Codex 런타임 라이프사이클 표준화 (`connect -> run/setup -> ask -> close -> shutdown`)
2. 스키마/JSON-RPC 계약 검증
3. 릴리즈 전 품질 게이트(포맷/린트/테스트/스키마/문서 동기화)

---

## 무엇이 해결되는가
Codex를 Rust에서 직접 붙일 때 반복되는 운영 문제를 줄입니다.

- 프로세스 spawn/shutdown, 세션 수명주기, 에러 매핑을 API로 고정
- agent ingress(`stdio/http/ws`)와 Rust dispatch(`CoclaiAgent`)를 단일 계약으로 고정
- Hook 체인(pre/post) 실패 시 메인 실행은 지속(fail-open)
- schema drift/manifest/doc-sync를 게이트로 운영

---

## 워크스페이스 구조

| 경로 | 역할 |
|---|---|
| `crates/coclai` | 단일 런타임 crate (domain/application/ports/adapters/bootstrap 포함) |
| `crates/coclai/src/bin/coclai_agent.rs` | 외부 1급 진입점 (`stdio/http/ws`) |
| `crates/coclai/src/agent` | transport-agnostic capability dispatch 경계 |
| `crates/coclai/src/domain|application|ports|adapters|infrastructure|bootstrap` | 헥사고날 경계 |
| `legacy/` 또는 제외된 레거시 crate | 아카이브 대상(비권장, 릴리즈 경로에서 제외) |
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
4. 패키지 기본 경로 (`crates/coclai/../../SCHEMAS/app-server/active`)

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

### 설치형 `coclai-agent` (macOS, launchd 기본)

1) 설치/업데이트(기본 채널)
```bash
./scripts/macos/install_coclai_agent.sh
```

2) 롤백(직전 바이너리 복구)
```bash
./scripts/macos/rollback_coclai_agent.sh
```

3) 제거
```bash
./scripts/macos/uninstall_coclai_agent.sh
```

드라이런:
```bash
./scripts/macos/install_coclai_agent.sh --dry-run --skip-launchctl
./scripts/macos/uninstall_coclai_agent.sh --dry-run --keep-binary
./scripts/macos/rollback_coclai_agent.sh --dry-run --skip-launchctl
```

기본 경로:
- 바이너리: `~/.local/bin/coclai-agent`
- launchd plist: `~/Library/LaunchAgents/io.coclai.agent.plist`
- 상태 디렉터리: `~/.coclai/agent` (`COCLAI_AGENT_STATE_DIR`로 override 가능)

### `coclai-agent` 운영 명령

```bash
# foreground 서비스 (개발/디버그)
coclai-agent start --foreground --bind 127.0.0.1:8787

# background 서비스 시작/중지
coclai-agent start --bind 127.0.0.1:8787
coclai-agent stop

# 상태/기능 조회
coclai-agent status
coclai-agent list-capabilities --ingress stdio
coclai-agent invoke system/health

# 네트워크 ingress 계약 검증(로컬 loopback + 토큰 필요)
COCLAI_AGENT_TOKEN=dev-token coclai-agent invoke system/health \
  --ingress http \
  --caller 127.0.0.1:39000 \
  --token dev-token

# HTTP ingress (axum)
curl -sS -H "x-coclai-token: dev-token" \
  http://127.0.0.1:8787/health
curl -sS -H "x-coclai-token: dev-token" \
  -H "content-type: application/json" \
  -d '{"capability_id":"system/capability_registry"}' \
  http://127.0.0.1:8787/invoke
```

네트워크 ingress 경로:
- `GET /health` (`system/health`)
- `GET /capabilities` (`system/capability_registry`)
- `POST /invoke` (임의 capability envelope)
- `GET /ws` (WebSocket JSON envelope)

바인드 주소 우선순위:
1. CLI `--bind <host:port>`
2. `COCLAI_AGENT_BIND_ADDR`
3. 기본값 `127.0.0.1:8787`

---

## 빠른 시작

### 1) Rust에서 직접 호출

```rust
use coclai::{CapabilityIngress, CapabilityInvocation, CoclaiAgent};
use serde_json::json;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let agent = CoclaiAgent::new();
    let out = agent.dispatch(CapabilityInvocation {
        capability_id: "quick_run".to_owned(),
        ingress: CapabilityIngress::Stdio,
        correlation_id: None,
        session_id: None,
        caller_addr: None,
        auth_token: None,
        payload: json!({
            "cwd": "/ABS/PATH/WORKDIR",
            "prompt": "이 디렉터리 핵심을 3줄로 요약해줘"
        }),
    })?;

    println!("{}", out.result["assistant_text"].as_str().unwrap_or_default());
    Ok(())
}
```

### 2) `coclai-agent` CLI 호출

```bash
coclai-agent status
coclai-agent list-capabilities --ingress stdio
coclai-agent invoke system/health
coclai-agent invoke quick_run --payload '{"cwd":"/ABS/PATH/WORKDIR","prompt":"요약해줘"}'
```

### 3) 실행 가능한 예제

```bash
coclai-agent invoke system/capability_parity_report
```

---

## 사용 경로 가이드

권장 경로:
1. 외부 통합: `coclai-agent` ingress (`stdio`, `http(localhost)`, `ws(localhost)`)
2. Rust 통합: `CoclaiAgent::dispatch(CapabilityInvocation)`

운영 원칙:
- capability 요청/응답 계약은 ingress별로 동일 의미를 유지합니다.
- 네트워크 ingress는 `loopback + token` 정책을 강제합니다.
- 0.2.0부터 공개면은 agent-first로 고정되며, 과거 broad facade 기반 통합은 권장하지 않습니다.
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

### `coclai-agent` 로컬 ingress 경계
- `http(localhost)`/`ws(localhost)` ingress는 loopback caller만 허용 (`--caller`가 `127.0.0.1`, `::1`, `localhost` 계열이어야 함)
- 네트워크 ingress는 토큰 일치가 필수 (`COCLAI_AGENT_TOKEN` == `--token`)
- 토큰 미설정 상태에서 네트워크 ingress 호출은 거부됨(기본 deny)
- `stdio` ingress는 로컬 프로세스 경로로 간주되어 토큰 요구 없음

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
4. workspace 테스트 (실서버 2건 제외)
5. `coclai` real-server 테스트 2건 별도 실행 (`quick_run`, `workflow_run`)
6. runtime real-cli contract (`contract_real_cli`)
7. schema drift (**hard-fail**, `source=codex`)
8. schema manifest
9. doc contract sync (**hard + mismatch=0 강제**)

실서버 재시도 제어:
- `COCLAI_RELEASE_REAL_SERVER_RETRIES` (기본 `3`, 최소 `1`)
- `COCLAI_RELEASE_REAL_SERVER_BACKOFF_SEC` (기본 `3`)

옵션 게이트:
- `COCLAI_RELEASE_INCLUDE_PERF=1` -> `run_micro_bench.sh` 포함
- `COCLAI_RELEASE_INCLUDE_NIGHTLY=1` -> `run_nightly_opt_in_gate.sh` 포함

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
- **0.2.0 브레이킹 릴리즈 기준**
- 외부 1급 통합 경로는 `coclai-agent` ingress(`stdio/http/ws`)로 고정
- 기존 Rust API는 보조 경로이며, 하위 호환을 보장하지 않음
- 권한 상승 경로 사용자는 `allow_privileged_escalation` 필요
- 릴리즈 게이트는 drift 발견 시 hard-fail
- 네트워크 ingress 소비자는 `COCLAI_AGENT_TOKEN` 발급/배포 절차를 먼저 준비해야 함

### Rollout Plan
1. 로컬 게이트 통과 (`fmt`, `clippy`, `test`, `doc-sync`)
2. `release_preflight.sh` 실행
3. `release_agent_go_no_go.sh` 실행 (lifecycle + local ingress security)
4. drift 해결 후 태그/릴리즈 진행

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
- Analysis Contract Matrix: `Docs/CONTRACT-MATRIX.md`

---

## 기여
- 변경 전 `cargo fmt --check`, `cargo clippy`, `cargo test --workspace` 실행 권장
- 계약/스키마 변경이 있으면 `scripts/*` 게이트까지 함께 확인
- 문서 주장 변경 시 `Docs/CONTRACT-MATRIX.md`와 동기화 유지

---

## 라이선스
MIT
