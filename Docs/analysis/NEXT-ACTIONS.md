# Next Actions (Post-Approval Backlog)

## Scope

- Completed in this step: **T-080**
- In scope artifacts:
  - `/Users/axient/repository/coclai/Docs/analysis/FINAL-ANALYSIS-REPORT.md`
  - `/Users/axient/repository/coclai/Docs/analysis/CONTRACT-MATRIX.md`
  - `/Users/axient/repository/coclai/Docs/analysis/EVIDENCE-MAP.md`
  - `/Users/axient/repository/coclai/Docs/TASKS.md`
  - `/Users/axient/repository/coclai/Docs/IMPLEMENTATION-PLAN.md`
- Out of scope in this step:
  - 실제 코드 구현 착수(본 문서는 승인 후 실행 큐와 게이트 확정만 수행)

## Snapshot Metadata

- Collected at (UTC): `2026-02-27T08:38:07Z`
- Baseline repo/branch: `coclai@dev`
- Prerequisite decision: `Option C (Contract-first staged refactor)` 승인됨

## Verification Map (Narrow -> Broader)

1. Narrow: 승인 직후 바로 집행 가능한 실행 단위를 고정한다.
2. Medium: 각 단위에 verify gate와 rollback trigger를 1:1로 연결한다.
3. Broader: 실행 순서(critical path)와 병렬 가능 구간을 명시해 운영 노이즈를 줄인다.
4. Cross-check: `T-071` 불변식/의존성 규칙과 `T-072` 단계 runbook을 직접 참조한다.

## T-080 Post-Approval Execution Backlog and Gates (DONE)

### A. Data Model (Execution Unit Schema)

실행 백로그 단위는 아래 필드로 고정한다.

1. `BacklogId` (`BL-001..`)
2. `SourceStage` (`R1..R6`, `SEC-*`, `QA-*`)
3. `Priority` (`P0|P1`)
4. `Preconditions`
5. `Action`
6. `VerifyGate`
7. `RollbackTrigger`
8. `RollbackAction`
9. `OutputArtifact`

### B. Approval Entry Gate (Start Criteria)

아래 4개가 모두 충족되어야 `BL-001` 착수를 허용한다.

1. 권고안 승인: `Docs/analysis/FINAL-ANALYSIS-REPORT.md`의 Option C 승인
2. 불변식 잠금: `I-01..I-06`, `D-01..D-05` 유지 확인
3. 베이스라인 무결성: `./scripts/check_schema_manifest.sh`, `cargo check -q --workspace` 통과
4. 작업 단위 규칙: 단계당 단일 PR(또는 독립 커밋) 원칙 적용

### C. Ordered Backlog (Execution Queue)

| BacklogId | SourceStage | Priority | Preconditions | Action | VerifyGate | RollbackTrigger | RollbackAction | OutputArtifact |
|---|---|---|---|---|---|---|---|---|
| BL-001 | `R1` | P0 | Entry Gate pass | method catalog 단일화(`appserver` constants + known-method source 동기화) | `cargo test -q -p coclai_runtime passes_unknown_method_in_known_mode` + method diff 0 | known-method 테스트 실패 또는 set diff 발생 | catalog 변경 커밋 revert 후 테스트 재실행 | `crates/coclai/src/appserver.rs`, `crates/coclai_runtime/src/rpc_contract.rs` |
| BL-002 | `R2` | P0 | BL-001 DONE | `api.rs` 분할(`models/ops/flow`), 공개 re-export 불변 유지 | `cargo test -q -p coclai_runtime run_prompt_hook_failure_is_fail_open_with_report` + `cargo test -q -p coclai_runtime session_config_hooks_register_and_execute` | hook/session 회귀 | `api` 분할 커밋 revert | `crates/coclai_runtime/src/api/*` |
| BL-003 | `R3` | P0 | BL-002 DONE | `client.rs` 분할(`config/session/profile/compat_guard`) | `cargo test -q -p coclai_runtime session_open_guards_return_error_when_closed` + `cargo test -q -p coclai_runtime session_close_keeps_local_handle_closed_when_archive_rpc_fails` + `cargo test -q -p coclai_runtime run_profile_defaults_are_explicit` | close/profile/default 회귀 | `client` 분할 커밋 revert | `crates/coclai_runtime/src/client/*` |
| BL-004 | `R4` | P0 | BL-003 DONE | runtime lifecycle/supervisor 상태머신 격리 | `cargo test -q -p coclai_runtime spawn_fails_when_manifest_mismatches_schema_files` + `cargo test -q -p coclai_runtime --test contract_deterministic` | schema/lifecycle 회귀 | runtime 분할 커밋 revert | `crates/coclai_runtime/src/runtime/*` |
| BL-005 | `SEC-004, SEC-007` | P0 | BL-001~BL-004 DONE | 보안 mismatch 폐쇄(권한 상승 강제 조건, 외부 `rpc_id` 비노출) | 보안 경계 회귀 테스트 + `cargo test -q -p coclai_web --lib` | tenant/approval/security regression | 보안 가드 변경 revert | `crates/coclai_runtime/src/*`, `crates/coclai_web/src/*`, 보강 테스트 |
| BL-006 | `R5` | P0 | BL-004 DONE | drift gate 추가(`scripts/check_schema_drift.sh`, preflight 연결) | drift 주입 시 fail + 정상 상태 pass | false-positive 과다 또는 릴리즈 차단 | ENV soft mode 전환 + 스크립트 revert | `scripts/check_schema_drift.sh`, `scripts/release_preflight.sh` |
| BL-007 | `R6` | P1 | BL-002~BL-006 DONE | 문서-코드 계약 동기화 체크 자동화 | 선언-구현 링크 커버리지 100% + `cargo check -q --workspace` | 문서 체크 오탐/미탐 | non-blocking report 모드 전환 | docs sync checker + `Docs/*` |
| BL-008 | `QA follow-up` | P1 | BL-001~BL-007 DONE | opt-in real CLI contract 및 script 테스트 공백 보강 | nightly/opt-in 게이트 실행 로그 + 스크립트 smoke harness 통과 | flaky 증가, CI 과부하 | 게이트 스코프 축소 후 재측정 | test job config, script tests |

### D. Critical Path and Parallelization

1. Critical path: `BL-001 -> BL-002 -> BL-003 -> BL-004 -> BL-006 -> BL-007`
2. `BL-005`는 구조 분할 완료 후(`BL-004`) 병행 가능
3. `BL-008`은 품질 보강 단계로 마지막에 병행 가능

### E. Merge Gates (Per Backlog Unit)

각 `BL-*`는 아래 공통 게이트를 만족해야 머지 가능하다.

1. 해당 `VerifyGate` 통과
2. `cargo check -q --workspace` 통과
3. `./scripts/check_schema_manifest.sh` 통과
4. 금지 의존성 유지:
   - `coclai_runtime -> coclai_artifact|coclai_web` 신규 edge 없어야 함

### F. Blocker Policy

아래 조건 발생 시 해당 `BL-*`를 `BLOCKED`로 전환한다.

1. 외부 의존(업스트림 schema/cli 버전) 변경으로 재현 불가
2. 반복 실패(동일 verify gate 2회 이상) + 원인 불명
3. 불변식(`I-02`, `I-04`, `I-06`) 또는 의존성(`D-05`) 위반

`BLOCKED` 시 필수 기록:
- owner/system
- 재검증 명령
- 로그/아티팩트 경로

### G. Transformations vs Side Effects

| Step | Transformation | Side Effect | Evidence |
|---|---|---|---|
| backlog normalization | `IA-*`와 `R1..R6`를 `BL-*` 실행 단위로 정규화 | none | 본 문서 C 섹션 |
| gate binding | 각 `BL-*`에 verify/rollback 트리거 바인딩 | none | 본 문서 C/E/F 섹션 |
| readiness check | 착수 기준과 blocker 정책을 실행 규칙으로 고정 | none | 본 문서 B/F 섹션 |

### H. Perf Notes

- 본 `T-080`은 실행 큐 확정 문서화 단계로 런타임 성능 변경은 없다.
- 실제 실행 비용은 `BL-002~BL-004`의 구조 분할 검증에 집중되며, 단계형 진행으로 실패 복구 비용을 제한한다.
- `BL-006~BL-008`은 스크립트/게이트 보강 중심으로 컴파일 비용보다 CI 운영비(실행 횟수) 영향이 크다.

### I. Deterministic Evidence Commands

```bash
# 실행 큐 근거 재확인
rg -n "Option C|Immediate Actions|IA-0" Docs/analysis/FINAL-ANALYSIS-REPORT.md
rg -n "R1|R2|R3|R4|R5|R6|VerifyGate|Rollback" Docs/analysis/NEXT-ACTIONS.md
rg -n "SEC-004|SEC-007|mismatch|uncertain|Total declarations evaluated: `78`" Docs/analysis/CONTRACT-MATRIX.md
rg -n "H0|opt-in|missing in active=59|hash-diff\\(common\\)=41" Docs/analysis/EVIDENCE-MAP.md

# 착수 기준 무결성 확인
./scripts/check_schema_manifest.sh
cargo check -q --workspace
```

### J. Task Sync Evidence

- Lifecycle: `T-080` `TODO -> DOING -> DONE`
- Artifact:
  - `Docs/TASKS.md`
  - `Docs/analysis/NEXT-ACTIONS.md#t-080-post-approval-execution-backlog-and-gates-done`

### K. Self-Verify Execution Evidence (T-080)

```bash
./scripts/check_schema_manifest.sh
# result: manifest OK

cargo check -q --workspace
# result: success (no diagnostics)
```

## BL-001 Execution Log (T-101 DONE)

### A. Data Model

- 목적: `rpc_methods`(facade 상수)와 `known method` 검증 집합의 source-of-truth를 단일화한다.
- 구현 단위:
  1. runtime canonical catalog 정의 (`coclai_runtime::rpc_contract::methods`)
  2. known-method 판별 로직이 catalog를 직접 사용
  3. facade `appserver::methods`는 runtime catalog를 재export

### B. Applied Changes

1. `crates/coclai_runtime/src/rpc_contract.rs`
   - `methods` 모듈 추가(`THREAD_*`, `TURN_*`, `KNOWN`)
   - request/response 검증 `match`가 문자열 리터럴 대신 `methods::*` 상수 사용
   - `is_known_method`를 `methods::KNOWN.contains(&method)`로 변경
   - 회귀 고정 테스트 `known_method_catalog_is_stable` 추가
2. `crates/coclai/src/appserver.rs`
   - 기존 중복 상수 정의 제거
   - `coclai_runtime::rpc_contract::methods::*` 재export로 단일 소스 참조

### C. Verify Gate Results

```bash
cargo test -q -p coclai_runtime known_method_catalog_is_stable
# result: ok. 1 passed

cargo test -q -p coclai_runtime passes_unknown_method_in_known_mode
# result: ok. 1 passed

cargo test -q -p coclai method_constants_are_stable
# result: ok. 1 passed

cargo test -q -p coclai appserver::tests::request_json_rejects_invalid_known_params
# result: ok. 1 passed

./scripts/check_schema_manifest.sh
# result: manifest OK

cargo test -q -p coclai_runtime spawn_fails_when_manifest_mismatches_schema_files
# result: ok (1 passed)

cargo test -q -p coclai_runtime --test contract_deterministic
# result: ok (3 passed)

cargo check -q --workspace
# result: success (no diagnostics)
```

### D. Transformations vs Side Effects

| Step | Transformation | Side Effect | Evidence |
|---|---|---|---|
| catalog extraction | method literal 집합을 runtime `methods::KNOWN`으로 정규화 | none | `rpc_contract.rs` |
| facade sync | appserver 상수를 runtime catalog 재export로 치환 | none | `appserver.rs` |
| regression verify | known/pass-through/typed validation 회귀 확인 | 테스트/체크 실행 | Verify Gate Results |

### E. Perf Notes

- 본 단계는 상수 참조 구조 변경이며 RPC 호출 경로의 알고리즘 복잡도 변화는 없다.
- `is_known_method`는 고정 길이(10) 배열 membership 검사로 기존 `matches!`와 동급 비용이다.

## BL-002 Execution Log (T-102 DONE)

### A. Data Model

- 목적: `api.rs`를 `models/ops/flow` 경계로 1차 분할하면서 공개 API 경로를 유지한다.
- 제약:
  1. `coclai_runtime::api::*` 공개 타입/함수 경로 불변
  2. hook/session 핵심 회귀 테스트 통과
  3. 단계적 분할(대규모 이동 대신 경계 도입 우선)

### B. Applied Changes

1. 새 모듈 추가
   - `crates/coclai_runtime/src/api/models.rs`
   - `crates/coclai_runtime/src/api/ops.rs`
   - `crates/coclai_runtime/src/api/flow.rs`
2. `api.rs` 경계 도입
   - `mod models; mod ops; mod flow;`
   - `pub use models::{PromptRunParams, PromptRunResult, PromptRunError, ...}` 재노출
3. 분리된 책임
   - `models`: prompt-run 모델/에러 타입
   - `ops`: `ThreadHandle` 조작 + serialize/deserialize helper
   - `flow`: hook mutation/context/state helper + lag 보조 로직 + best-effort interrupt
4. 테스트 가시성 유지
   - `api/tests.rs`가 참조하는 wire helper는 `#[cfg(test)]` import로 유지

### C. Verify Gate Results

```bash
cargo test -q -p coclai_runtime run_prompt_hook_failure_is_fail_open_with_report
# result: ok. 1 passed

cargo test -q -p coclai_runtime session_config_hooks_register_and_execute
# result: ok. 1 passed

cargo test -q -p coclai --lib
# result: ok. 17 passed

./scripts/check_schema_manifest.sh
# result: manifest OK

cargo check -q --workspace
# result: success (no diagnostics)
```

### D. Transformations vs Side Effects

| Step | Transformation | Side Effect | Evidence |
|---|---|---|---|
| module split | `api.rs` 내부 책임을 `models/ops/flow`로 분리 | none | `api.rs`, `api/models.rs`, `api/ops.rs`, `api/flow.rs` |
| API path guard | 공개 타입을 `pub use models::*`로 재노출 | none | `api.rs` re-export |
| regression verify | hook/session/public facade 회귀 확인 | 테스트/체크 실행 | Verify Gate Results |

### E. Perf Notes

- 본 단계는 구조 분할이며 runtime 핫패스 알고리즘은 유지된다.
- 추가 모듈 경계는 컴파일 단위 분리 효과가 있으나 실행 경로 비용 변화는 실질적으로 없다.

## BL-003 Execution Log (T-103 DONE)

### A. Data Model

- 목적: `client.rs`를 `config/session/profile/compat_guard` 경계로 1차 분할한다.
- 제약:
  1. 공개 타입/API 경로(`Client`, `ClientConfig`, `Session`, `RunProfile`, `SessionConfig`) 유지
  2. 세션 close/guard/profile 기본값 불변식 유지
  3. 기존 `client/tests.rs` 회귀 없이 통과

### B. Applied Changes

1. 새 모듈 추가
   - `crates/coclai_runtime/src/client/config.rs`
   - `crates/coclai_runtime/src/client/profile.rs`
   - `crates/coclai_runtime/src/client/session.rs`
   - `crates/coclai_runtime/src/client/compat_guard.rs`
2. `client.rs` 재구성
   - 조립/공개 경계(`pub use`) + `Client` 본체 + `ClientError` 중심으로 축소
   - 내부 분리 모듈 함수 사용으로 책임 분산
3. 테스트 호환성 유지
   - `client/tests.rs`가 사용하는 내부 helper 시그니처를 유지하기 위해 테스트 전용 래퍼(`#[cfg(test)]`) 제공
4. 생성 책임 정리
   - `Session` 생성은 `Session::new`(`pub(super)`)로 통일해 분할 후 생성 경계 명확화

### C. Verify Gate Results

```bash
cargo test -q -p coclai_runtime session_open_guards_return_error_when_closed
# result: ok. 1 passed

cargo test -q -p coclai_runtime session_close_keeps_local_handle_closed_when_archive_rpc_fails
# result: ok. 1 passed

cargo test -q -p coclai_runtime run_profile_defaults_are_explicit
# result: ok. 1 passed

cargo test -q -p coclai_runtime client::tests::
# result: ok. 20 passed

./scripts/check_schema_manifest.sh
# result: manifest OK

cargo check -q --workspace
# result: success (no diagnostics)
```

### D. Transformations vs Side Effects

| Step | Transformation | Side Effect | Evidence |
|---|---|---|---|
| module split | `client.rs` 책임을 `config/profile/session/compat_guard`로 분리 | none | 신규 4개 모듈 + `client.rs` 재구성 |
| API path guard | 기존 공개 타입/함수 경로 유지 (`pub use`) | none | `client.rs` exports |
| regression verify | session/profile/compatibility 핵심 회귀 확인 | 테스트/체크 실행 | Verify Gate Results |

### E. Perf Notes

- 본 단계는 구조 분해이며 런타임 RPC 경로 알고리즘 변경은 없다.
- 분리 후 컴파일 단위가 작아져 후속 변경의 빌드 영향 범위를 줄이는 효과가 있다.

## BL-004 Execution Log (T-104 DONE)

### A. Data Model

- 목적: runtime lifecycle/supervisor 상태머신 오케스트레이션을 `runtime.rs`에서 분리해 경계를 명시한다.
- 제약:
  1. spawn/shutdown/restart 동작 불변
  2. schema fail-fast 및 contract deterministic 회귀 유지
  3. `coclai_runtime -> coclai_artifact|coclai_web` 역의존 금지 유지

### B. Applied Changes

1. `crates/coclai_runtime/src/runtime/lifecycle.rs`
   - generation teardown 공통 경로를 `teardown_generation(Detach|Shutdown)`으로 통합
   - shutdown 상태 전이/태스크 join 오케스트레이션을 `shutdown_runtime`로 추출
2. `crates/coclai_runtime/src/runtime/supervisor.rs`
   - supervisor spawn/handle 저장 경계를 `start_supervisor_task` helper로 추출
3. `crates/coclai_runtime/src/runtime.rs`
   - `spawn_local`이 `start_supervisor_task`를 사용하도록 변경
   - `shutdown` 본문을 `shutdown_runtime` 호출로 치환
   - 중복된 transport/dispatcher/pending teardown 절차 제거

Lifecycle sync: `T-104 TODO -> DOING -> DONE`

### C. Verify Gate Results

```bash
cargo test -q -p coclai_runtime runtime::tests::spawn_fails_when_manifest_mismatches_schema_files
# result: ok. 1 passed

cargo test -q -p coclai_runtime --test contract_deterministic
# result: ok. 3 passed

./scripts/check_schema_manifest.sh
# result: manifest OK

cargo check -q --workspace
# result: success (no diagnostics)
```

### D. Transformations vs Side Effects

| Step | Transformation | Side Effect | Evidence |
|---|---|---|---|
| lifecycle shutdown isolation | shutdown/detach teardown 상태머신을 lifecycle 모듈로 이동 | none | `runtime/lifecycle.rs`, `runtime.rs` |
| supervisor spawn isolation | supervisor task 시작/등록을 supervisor 모듈 helper로 이동 | none | `runtime/supervisor.rs`, `runtime.rs` |
| regression verify | schema fail-fast + contract deterministic + workspace gate 확인 | 테스트/체크 실행 | Verify Gate Results |

### E. Perf Notes

- 본 단계는 상태머신 오케스트레이션 분리이며 RPC hot path 알고리즘/복잡도 변화는 없다.
- teardown 중복 제거로 종료 경로 유지보수 비용과 변경 파급 범위를 줄였다.

## BL-005 Execution Log (T-105 DONE)

### A. Data Model

- 목적: `SEC-004`/`SEC-007` mismatch를 코드 레벨에서 폐쇄한다.
- 제약:
  1. 기본 보안값(`approval=never`, `sandbox=readOnly`) 유지
  2. 권한 상승은 명시적 opt-in + scope + 승인경로 조건 모두 만족 시에만 허용
  3. 외부 SSE payload에서 내부 `rpc_id` 비노출

### B. Applied Changes

1. `crates/coclai_runtime/src/api/*` 보안 가드 추가
   - `ThreadStartParams`/`TurnStartParams`에 `privileged_escalation_approved` 필드 추가
   - `PromptRunParams`/`RunProfile`/`SessionConfig`에 동일 필드 및 opt-in builder 추가
   - `validate_thread_start_security`/`validate_turn_start_security`를 추가해 privileged sandbox 사용 시 아래를 강제:
     - explicit opt-in
     - `approval_policy != never`
     - scope 명시(`cwd` 또는 `writableRoots`)
2. `crates/coclai_web/src/wire.rs` SSE 직렬화 강화
   - `serialize_sse_envelope`에서 top-level `rpcId` 제거
   - `kind=response|unknown` payload의 `json.id` 제거로 내부 RPC 식별자 비노출 보강
3. 테스트 보강
   - runtime: privileged sandbox guard 거절/허용 경로 테스트 추가
   - web: SSE 직렬화에서 `rpcId` 누락 검증 추가

Lifecycle sync: `T-105 TODO -> DOING -> DONE`

### C. Verify Gate Results

```bash
cargo test -q -p coclai_runtime privileged_sandbox
# result: ok. 4 passed

cargo test -q -p coclai_runtime prompt_run_params_builder_overrides_defaults
# result: ok. 1 passed

cargo test -q -p coclai_runtime session_config_from_profile_maps_all_fields
# result: ok. 1 passed

cargo test -q -p coclai_web serialize_envelope_to_sse
# result: ok. 1 passed

cargo test -q -p coclai_web --lib
# result: ok. 13 passed

cargo test -q -p coclai_runtime --test contract_deterministic
# result: ok. 3 passed

cargo test -q -p coclai_runtime --lib
# result: ok. 138 passed

./scripts/check_schema_manifest.sh
# result: manifest OK

cargo check -q --workspace
# result: success (no diagnostics)
```

### D. Transformations vs Side Effects

| Step | Transformation | Side Effect | Evidence |
|---|---|---|---|
| escalation guard | privileged sandbox 경로를 explicit opt-in + scope + approval 조건으로 제한 | invalid 설정 시 즉시 `InvalidRequest` 반환 | `api/wire.rs`, `api.rs`, `api/ops.rs` |
| profile propagation | run/session/profile 경계에 escalation 승인 플래그 전파 | none | `api/models.rs`, `client/profile.rs` |
| external id redaction | SSE payload 직렬화에서 `rpcId` 제거 | none | `coclai_web/src/wire.rs`, `coclai_web/src/tests.rs` |
| regression verify | runtime/web 보안 회귀 + workspace gate 확인 | 테스트/체크 실행 | Verify Gate Results |

### E. Perf Notes

- 본 단계는 검증 분기/직렬화 정제 추가로 핫패스 복잡도 변화는 크지 않다.
- 보안 검증은 O(1) 분기 + 소규모 필드 검사(문자열 trim/배열 순회) 수준이다.

## BL-006 Execution Log (T-106 DONE)

### A. Data Model

- 목적: active schema와 현재 generator 결과의 external parity drift를 gate에서 노출한다.
- 제약:
  1. preflight에 drift gate를 연결하되 운영 노이즈를 줄이기 위해 soft fallback 유지
  2. drift 비교는 legacy prune 정책을 동일하게 적용한 후 수행
  3. 드리프트 주입 시 deterministic하게 실패 재현 가능해야 함

### B. Applied Changes

1. 공통 prune helper 도입
   - `scripts/prune_schema_legacy.sh` 추가
   - `scripts/update_schema.sh`의 legacy prune 하드코딩 목록을 helper 호출로 대체
2. drift gate 스크립트 추가
   - `scripts/check_schema_drift.sh`
   - 기본 모드: `COCLAI_SCHEMA_DRIFT_MODE=soft` (`hard|soft|off`)
   - 비교 소스: `COCLAI_SCHEMA_DRIFT_SOURCE=codex|active` (`codex` 기본)
   - 테스트 주입: `COCLAI_SCHEMA_DRIFT_INJECT=1`이면 deterministic drift 파일 추가
3. preflight 연동
   - `scripts/release_preflight.sh`에 `schema drift` gate 추가(`COCLAI_SCHEMA_DRIFT_SOURCE=codex` 고정)
   - 실행 순서: `tests -> schema drift -> schema manifest`

Lifecycle sync: `T-106 TODO -> DOING -> DONE`

### C. Verify Gate Results

```bash
./scripts/check_schema_drift.sh
# result: soft mode pass (drift warning surfaced)

COCLAI_SCHEMA_DRIFT_MODE=hard COCLAI_SCHEMA_DRIFT_SOURCE=active ./scripts/check_schema_drift.sh
# result: ok (normal hard pass on self-baseline)

COCLAI_SCHEMA_DRIFT_MODE=hard COCLAI_SCHEMA_DRIFT_SOURCE=active COCLAI_SCHEMA_DRIFT_INJECT=1 ./scripts/check_schema_drift.sh
# result: fail (deterministic injected drift detected)

./scripts/release_preflight.sh
# result: preflight passed (drift gate connected and executed)

./scripts/check_schema_manifest.sh
# result: manifest OK

cargo check -q --workspace
# result: success (no diagnostics)
```

### D. Transformations vs Side Effects

| Step | Transformation | Side Effect | Evidence |
|---|---|---|---|
| prune unification | schema prune 정책을 단일 helper로 정규화 | none | `scripts/prune_schema_legacy.sh`, `scripts/update_schema.sh` |
| drift detection gate | generated vs active 파일셋/해시 drift를 gate로 노출 | soft mode에서는 warning 출력 | `scripts/check_schema_drift.sh` |
| preflight integration | 릴리즈 preflight에 drift gate 추가 | preflight 출력에 drift summary 포함 | `scripts/release_preflight.sh` 실행 로그 |

### E. Perf Notes

- drift 비교는 파일 수 `n`에 대해 정렬/비교 중심 `O(n log n)`이며 현재 schema 규모에서 실행 시간은 짧다(약 1초 내외).
- preflight 추가 비용은 schema 생성 단계가 대부분이며, 기존 `fmt/clippy/test` 비용 대비 상대적으로 작다.

## BL-007 Execution Log (T-107 DONE)

### A. Data Model

- 목적: 문서 선언과 구현 근거의 링크 커버리지를 자동으로 검증한다.
- 제약:
  1. 대상은 `Docs/analysis/CONTRACT-MATRIX.md`의 `Declaration vs Implementation Matrix`로 고정
  2. `Implementation Evidence` 누락(`-`/empty) 항목이 1개라도 있으면 실패
  3. rollback을 위해 non-blocking 모드(`COCLAI_DOC_SYNC_MODE=soft`) fallback 제공

### B. Applied Changes

1. 동기화 체크 스크립트 추가
   - `scripts/check_doc_contract_sync.sh`
   - 모드: `COCLAI_DOC_SYNC_MODE=hard|soft|off` (`hard` 기본)
   - 대상 문서 override: `COCLAI_DOC_CONTRACT_MAP=<path>`
2. 검증 로직
   - matrix 행(`CON/ARC/API/SCC/SEC`) 파싱 후 `Implementation Evidence` 커버리지 계산
   - 커버리지/판정 카운트(`match/mismatch/uncertain`) 출력
   - `T-022 Summary` 수치와 계산 수치 불일치 시 실패
3. preflight 연동
   - `scripts/release_preflight.sh`에 `doc contract sync` gate 추가
4. 사용자 문서 동기화
   - `README.md`와 `Docs/SCHEMA_AND_CONTRACT.md`에 doc-sync 체크 명령 추가

Lifecycle sync: `T-107 TODO -> DOING -> DONE`

### C. Verify Gate Results

```bash
./scripts/check_doc_contract_sync.sh
# result: coverage total=78 linked=78 pct=100.00%, OK

COCLAI_DOC_SYNC_MODE=hard COCLAI_DOC_CONTRACT_MAP=<injected-matrix> ./scripts/check_doc_contract_sync.sh
# result: fail (missing implementation evidence detected)

./scripts/release_preflight.sh
# result: preflight passed (doc contract sync gate connected and executed)

./scripts/check_schema_manifest.sh
# result: manifest OK

cargo check -q --workspace
# result: success (no diagnostics)
```

### D. Transformations vs Side Effects

| Step | Transformation | Side Effect | Evidence |
|---|---|---|---|
| matrix coverage gate | declaration-implementation 링크 커버리지를 자동 계산/검증 | coverage 미달 시 gate fail | `scripts/check_doc_contract_sync.sh` |
| preflight integration | 릴리즈 preflight에 doc-sync gate 추가 | preflight 출력에 coverage 요약 포함 | `scripts/release_preflight.sh` 실행 로그 |
| rollback fallback | doc-sync 모드(`hard/soft/off`)로 차단 강도 조절 | soft 모드에서는 warning-only | `COCLAI_DOC_SYNC_MODE` 분기 |

### E. Perf Notes

- doc-sync 검사는 markdown 행 파싱 중심 `O(n)` (`n=matrix rows`)이며 현재 규모(78행)에서 실행 비용이 매우 작다.
- preflight 추가 비용은 컴파일/테스트 단계 대비 무시 가능한 수준이다.

## BL-008 Execution Log (T-108 DONE)

### A. Data Model

- 목적: `opt-in real CLI contract`와 `scripts/*` 주요 분기 검증을 지속 가능한 게이트로 고정한다.
- 제약:
  1. 기본 CI는 스크립트 smoke를 자동 수행해야 한다.
  2. real CLI 계약 검증은 opt-in/nightly lane으로 분리한다.
  3. 실행 로그는 재검증 가능한 파일 경로로 남긴다.

### B. Applied Changes

1. 스크립트 smoke harness 추가
   - `scripts/smoke_script_harness.sh`
   - 포함 검증:
     - `check_schema_manifest` 정상
     - `check_schema_drift` self-baseline 정상
     - `check_doc_contract_sync` 정상
     - schema/doc 주입 실패 경로(non-zero) 확인
2. nightly/opt-in 통합 게이트 추가
   - `scripts/run_nightly_opt_in_gate.sh`
   - 순서:
     - smoke harness 실행
     - `APP_SERVER_CONTRACT=1 cargo test -p coclai_runtime --test contract_real_cli -- --nocapture`
   - skip guard:
     - base skip 문구(`set APP_SERVER_CONTRACT=1`)가 로그에 있으면 실패 처리
3. test job config 보강
   - `ci.yml`에 `script smoke harness` step 추가
   - `.github/workflows/nightly-opt-in.yml` 추가
     - nightly `script-smoke` job
     - opt-in `real-cli-opt-in` job(`vars.COCLAI_ENABLE_REAL_CLI_NIGHTLY == '1'`, self-hosted)
4. 문서 동기화
   - `README.md`, `Docs/SCHEMA_AND_CONTRACT.md`에 smoke/nightly 게이트 명령 및 로그 경로 반영

Lifecycle sync: `T-108 TODO -> DOING -> DONE`

### C. Verify Gate Results

```bash
./scripts/smoke_script_harness.sh
# result: passed

./scripts/run_nightly_opt_in_gate.sh
# result: passed
# log_dir: target/qa/nightly_opt_in/20260227T095943Z

./scripts/check_schema_manifest.sh
# result: manifest OK

cargo check -q --workspace
# result: success (no diagnostics)
```

### D. Transformations vs Side Effects

| Step | Transformation | Side Effect | Evidence |
|---|---|---|---|
| script smoke harness | scripts 주요 정상/실패 분기 smoke 자동화 | `/tmp/coclai_smoke_*` 임시 로그 생성 | `scripts/smoke_script_harness.sh` |
| opt-in nightly lane | real CLI contract lane를 smoke와 결합한 단일 실행 스크립트 제공 | `target/qa/nightly_opt_in/*` 로그 생성 | `scripts/run_nightly_opt_in_gate.sh` |
| CI config split | 기본 CI와 nightly/opt-in lane 분리 운영 | nightly self-hosted 의존 lane 추가 | `.github/workflows/ci.yml`, `.github/workflows/nightly-opt-in.yml` |

### E. Perf Notes

- smoke harness는 O(1) 개수의 스크립트 호출과 소규모 파일 변조 검증만 수행한다.
- nightly lane 비용의 대부분은 real CLI contract 테스트 시간이며, opt-in 분리로 기본 CI 노이즈를 억제한다.

## BL-009 Execution Log (T-109~T-111 DONE)

### A. Data Model

- 목적: 외부 공개 기준에서 기본 예제 실패를 제거하고, 릴리즈 게이트를 fail-closed로 강화한다.
- 제약:
  1. 기본 전문가 경로는 privileged escalation 없이 동작해야 한다.
  2. privileged sandbox 예제는 explicit opt-in API를 명시해야 한다.
  3. `rpc_direct` 예제는 `inProgress` 출력으로 끝나지 않고 최종 텍스트를 수집해야 한다.
  4. 릴리즈 preflight의 schema drift는 warning-only가 아닌 hard-fail이어야 한다.

### B. Applied Changes

1. Workflow 예제 분리
   - `crates/coclai/examples/workflow.rs`를 safe default(`read-only`) 경로로 단순화
   - `crates/coclai/examples/workflow_privileged.rs` 신규 추가
     - `RunProfile::allow_privileged_escalation()` 명시
2. RPC direct 예제 완료 수집
   - `crates/coclai/examples/rpc_direct.rs`에서 `Runtime::subscribe_live()` 사용
   - `AssistantTextCollector`로 `turn/completed`까지 이벤트를 수집하고 최종 텍스트 출력
3. Release gate 강화
   - `scripts/release_preflight.sh`의 schema drift 실행을 `COCLAI_SCHEMA_DRIFT_MODE=hard`로 상향
4. 문서 동기화
   - `README.md`: safe/privileged workflow 예제 경로와 hard preflight 정책 반영
   - `Docs/CORE_API.md`: 실행 예제 2경로(safe/privileged) 반영
   - `Docs/SCHEMA_AND_CONTRACT.md`: preflight hard drift 정책 반영

Lifecycle sync:
- `T-109 TODO -> DOING -> DONE`
- `T-110 TODO -> DOING -> DONE`
- `T-111 TODO -> DOING -> DONE`

### C. Verify Gate Results

```bash
cargo run -q -p coclai --example workflow
# result: success (safe default path runs without privileged escalation opt-in)

cargo run -q -p coclai --example workflow_privileged
# result: success (explicit allow_privileged_escalation path)

cargo run -q -p coclai --example rpc_direct
# result: success (waits for turn/completed and prints final assistant text)

COCLAI_SCHEMA_DRIFT_MODE=hard COCLAI_SCHEMA_DRIFT_SOURCE=codex ./scripts/check_schema_drift.sh
# result: fail as expected when drift exists (release blocker)

cargo test -q -p coclai --lib
# result: pass

cargo check -q --workspace
# result: success (no diagnostics)
```

### D. Transformations vs Side Effects

| Step | Transformation | Side Effect | Evidence |
|---|---|---|---|
| safe/privileged split | workflow example을 safe default와 explicit privileged path로 분리 | none | `crates/coclai/examples/workflow.rs`, `crates/coclai/examples/workflow_privileged.rs` |
| turn completion collection | rpc_direct에서 live 이벤트를 수집해 최종 텍스트를 구성 | 이벤트 구독 루프 추가 | `crates/coclai/examples/rpc_direct.rs` |
| hard release drift gate | preflight drift 검사를 fail-closed로 전환 | drift 존재 시 preflight 즉시 실패 | `scripts/release_preflight.sh` |
| contract docs sync | 실행/정책 문서와 구현 경로 일치화 | none | `README.md`, `Docs/CORE_API.md`, `Docs/SCHEMA_AND_CONTRACT.md` |

### E. Perf Notes

- `workflow`/`workflow_privileged` 분리는 예제 계층 변경으로 런타임 핫패스에 영향이 없다.
- `rpc_direct` 완료 수집은 이벤트 수신 수 `n`에 대해 O(n)이며 예제 목적의 제어 루프다.

## BL-010 Execution Log (T-112 DONE)

### A. Data Model

- 목적: `SCHEMAS/app-server/active/json-schema`를 최신 generator 기준으로 동기화해 release hard gate에서 drift를 제거한다.
- 제약:
  1. runtime startup guard가 참조하는 `metadata.json`, `manifest.sha256`, `json-schema/*`의 무결성을 동시에 유지해야 한다.
  2. prune 정책(`scripts/prune_schema_legacy.sh`)은 기존과 동일하게 적용한다.
  3. 결과 검증은 `hard` 모드 drift gate로 고정한다.

### B. Applied Changes

1. active schema 재생성
   - `./scripts/update_schema.sh` 실행
   - 생성 결과를 `SCHEMAS/app-server/active/json-schema`로 교체 후 legacy prune 적용
2. legacy active-only 파일 제거
   - `v2/CollaborationModeListParams.json`
   - `v2/CollaborationModeListResponse.json`
3. 신규 generator 산출물 반영
   - 신규 파일(예: `ChatgptAuthTokensRefresh*.json`, `DynamicToolCall*.json`, `v2/TurnSteer*.json`, `v2/ThreadUnarchive*.json` 등) 추가
4. 메타/매니페스트 갱신
   - `SCHEMAS/app-server/active/metadata.json` `generatedAtUtc` 갱신
   - `SCHEMAS/app-server/active/manifest.sha256` 재작성

Lifecycle sync: `T-112 TODO -> DOING -> DONE`

### C. Verify Gate Results

```bash
COCLAI_SCHEMA_DRIFT_MODE=hard COCLAI_SCHEMA_DRIFT_SOURCE=codex ./scripts/check_schema_drift.sh
# before update: fail (missing=39 extra=2 hash_diff=41)
# after update: [schema-drift] OK (mode=hard, source=codex)

./scripts/check_schema_manifest.sh
# result: manifest OK

cargo test -q -p coclai_runtime spawn_fails_when_manifest_mismatches_schema_files
# result: ok (1 passed)

cargo test -q -p coclai_runtime --test contract_deterministic
# result: ok (3 passed)

cargo check -q --workspace
# result: success (no diagnostics)
```

### D. Transformations vs Side Effects

| Step | Transformation | Side Effect | Evidence |
|---|---|---|---|
| schema regeneration | active schema를 최신 generator output으로 재동기화 | `SCHEMAS/app-server/active/json-schema/*` 대량 변경 | `./scripts/update_schema.sh` 실행 로그 |
| prune alignment | legacy prune 정책으로 obsolete 파일 제거 | `CollaborationModeList*` 삭제 | `scripts/prune_schema_legacy.sh` + git diff |
| release gate hardening outcome | hard drift gate를 실제 최신 상태에서 통과 가능한 상태로 복구 | none | `check_schema_drift(hard,codex)` OK |

### E. Perf Notes

- 본 변경은 정적 schema 자산 교체 중심이며 런타임 실행 경로 알고리즘 변화는 없다.
- 영향 비용은 mostly CI/검증 단계의 파일 해시 비교(`O(n log n)`)이며 현재 스키마 규모에서 운영 가능 범위다.

## BL-011 Execution Log (T-113 DONE)

### A. Data Model

- 목적: 외부 공개 직전의 문서-게이트 정합성을 복구해 release 판정 기준을 단일화한다.
- 제약:
  1. `CONTRACT-MATRIX`의 stale mismatch(`SEC-004`, `SEC-007`)를 실제 코드/테스트 근거로 업데이트해야 한다.
  2. `release_preflight`의 doc-sync는 mismatch 허용이 아닌 strict 모드로 동작해야 한다.
  3. README/SCHEMA 문서의 stop/go 문구와 실제 gate 동작이 일치해야 한다.

### B. Applied Changes

1. contract matrix 최신화
   - `Docs/analysis/CONTRACT-MATRIX.md`
   - `SEC-004`, `SEC-007`: `mismatch -> match`
   - summary 갱신: `match=66`, `mismatch=0`, `uncertain=12`
2. doc-sync strict mismatch gate 도입
   - `scripts/check_doc_contract_sync.sh`
   - `COCLAI_DOC_SYNC_FAIL_ON_MISMATCH=0|1` 옵션 추가 (`1`이면 mismatch verdict 존재 시 fail)
3. release/smoke gate 동기화
   - `scripts/release_preflight.sh`: `COCLAI_DOC_SYNC_FAIL_ON_MISMATCH=1` 강제
   - `scripts/smoke_script_harness.sh`: 동일 strict 옵션으로 smoke gate 정합
4. 운영 문서 동기화
   - `README.md`: doc-sync strict 옵션, preflight 항목, stop/go 기준 갱신
   - `Docs/SCHEMA_AND_CONTRACT.md`: strict mismatch 옵션 및 preflight 강제 조건 반영

Lifecycle sync: `T-113 TODO -> DOING -> DONE`

### C. Verify Gate Results

```bash
bash -n scripts/check_doc_contract_sync.sh scripts/release_preflight.sh scripts/smoke_script_harness.sh
# result: OK (shell syntax valid)

COCLAI_DOC_SYNC_MODE=hard ./scripts/check_doc_contract_sync.sh
# result: coverage=100%, verdicts match=66 mismatch=0 uncertain=12, OK

COCLAI_DOC_SYNC_MODE=hard COCLAI_DOC_SYNC_FAIL_ON_MISMATCH=1 ./scripts/check_doc_contract_sync.sh
# result: coverage=100%, verdicts match=66 mismatch=0 uncertain=12, OK

COCLAI_DOC_SYNC_MODE=hard COCLAI_DOC_SYNC_FAIL_ON_MISMATCH=1 COCLAI_DOC_CONTRACT_MAP=<mismatch-injected-map> ./scripts/check_doc_contract_sync.sh
# result: fail as expected (strict mismatch gate blocks release)

./scripts/smoke_script_harness.sh
# result: passed

./scripts/release_preflight.sh
# result: preflight passed
```

### D. Transformations vs Side Effects

| Step | Transformation | Side Effect | Evidence |
|---|---|---|---|
| matrix refresh | stale security mismatch를 코드 현행 근거로 동기화 | none | `Docs/analysis/CONTRACT-MATRIX.md` |
| strict gate option | doc-sync에 mismatch 차단 모드를 추가 | strict 모드에서 mismatch 존재 시 non-zero exit | `scripts/check_doc_contract_sync.sh` |
| release alignment | preflight/smoke가 strict doc-sync 정책을 공통 사용 | 릴리즈 차단 조건 강화 | `scripts/release_preflight.sh`, `scripts/smoke_script_harness.sh` |
| docs alignment | README/SCHEMA 문서의 stop/go 문구를 gate 동작과 일치화 | none | `README.md`, `Docs/SCHEMA_AND_CONTRACT.md` |

### E. Perf Notes

- 본 단계는 문서/스크립트 정책 정합화로, 런타임 핫패스(`request/dispatch/state reduce`)의 실행 복잡도 변화는 없다.
- 추가 비용은 preflight에서 doc-sync mismatch 검사 1회(`O(n)`, `n=matrix rows`)이며 현재 78행 기준 영향이 작다.
