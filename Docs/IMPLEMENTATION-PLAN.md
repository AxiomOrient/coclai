# IMPLEMENTATION PLAN (Big-Bang Rewrite)

작성일: 2026-02-28
전략: Runtime/Web/Artifact 빅뱅 재작성

## Goal

`runtime/web/artifact` 구조를 단일 컷오버 방식으로 재작성하여 다음을 동시에 달성한다.

1. 공개 API(`coclai::quick_run`, `Workflow`, `AppServer`, `coclai_runtime` 공개 타입) 호환성을 유지한다.
2. 내부 구조를 책임 단위로 재배치해 유지보수성과 변경 안정성을 높인다.
3. 재작성 완료 시점에 기능 회귀/계약 위반/보안 회귀가 없음을 증거 기반으로 입증한다.

완료 기준:
- 전체 워크스페이스 테스트/정적 점검/릴리즈 프리플라이트 통과
- 공개 API snapshot diff 0
- `stub/TODO/unimplemented!/todo!()` 신규 잔존 0
- `runtime -> web|artifact` 역의존 0

## Scope

포함:
- `crates/coclai_runtime/src/**`
- `crates/coclai_web/src/**`
- `crates/coclai_artifact/src/**`
- `crates/coclai/src/{lib.rs,ergonomic.rs,appserver.rs}`
- `.github/workflows/ci.yml`, `scripts/{release_preflight.sh,check_*}`
- `README.md`, `Docs/CORE_API.md`, `Docs/SCHEMA_AND_CONTRACT.md`, `Docs/ARCHITECTURE.md`

제외:
- 신규 기능 추가
- 공개 API breaking change
- 신규 crate 도입
- 모델/프롬프트 정책 변경

## Constraints

- 사용자 지시사항 고정: 단계적 전환이 아니라 **빅뱅 컷오버**로 진행
- 구현 단계에서 임시 우회 코드(stub/feature-flag fallback) 금지
- 모든 변경은 재작성 브랜치에서 통합 후 단일 컷오버
- 병합 전 증거 없는 가정 금지(테스트/로그/명령 결과 필요)
- 과잉설계 금지: 추상화는 최소 2개 실사용 경로가 있을 때만 도입

## Data Model

현재 구조 관찰(증거 기반):
- 런타임 고복잡도 파일 집중
  - `api.rs` 1340 LOC
  - `runtime.rs` 598 LOC
  - `state.rs` 720 LOC
- 어댑터 고복잡도 파일 집중
  - `coclai_web/src/lib.rs` 387 LOC
  - `coclai_artifact/src/lib.rs` 466 LOC
- 의존 방향은 현재 양호
  - `coclai_web -> coclai_runtime`
  - `coclai_artifact -> coclai_runtime`

재작성 목표 모델:
- Runtime: API 파사드 / 실행 코어 / 상태 리듀서 / transport/rpc 경계를 명시 분리
- Web: 세션/턴/승인 라우팅 분리 + tenant ownership 규칙 명시
- Artifact: lifecycle/orchestration/store/patch 책임 분리
- Facade: 초보자 경로와 고급 경로를 동일 코드베이스에서 동시 제공

## Option Comparison (3)

### Option A. 유지보수 중심 점진 개선
- 장점: 리스크 낮음
- 단점: 사용자가 요구한 빅뱅 전략과 불일치

### Option B. 빅뱅 재작성(채택)
- 장점: 구조 일관성을 한 번에 확보, 중간 상태 복잡도 최소화
- 단점: 통합 리스크가 큼, 테스트/게이트 실패 시 롤백 비용 큼

### Option C. 신규 병행 구현 후 장기 이중운영
- 장점: 안정적 전환
- 단점: 중복 코드/운영비 증가, 과잉설계 위험

## Decision

`Option B (빅뱅 재작성)`을 채택한다.

채택 이유:
- 사용자 요구사항이 빅뱅 전략을 명시했고, 본 계획은 해당 제약을 만족해야 한다.
- 현재 테스트/검증 스크립트 자산이 충분하여 컷오버 품질 게이트를 강하게 설정할 수 있다.
- 단계적 전환 중간상태 비용(이중 경로 유지)을 제거할 수 있다.

## Target File Cut Map (Big-Bang)

Runtime (`crates/coclai_runtime/src`):
- `api.rs`(public facade + type contracts) + `api/{flow.rs,models.rs,ops.rs,wire.rs,turn_error.rs}`
- `runtime.rs`(runtime core facade) + `runtime/{dispatch.rs,lifecycle.rs,rpc_io.rs,state_projection.rs,supervisor.rs,tests.rs}`
- `state.rs`(state model + reducer + prune 경계 유지)
- 유지: `rpc.rs`, `rpc_contract.rs`, `transport.rs`, `turn_output.rs`, `hooks.rs`, `metrics.rs`, `approvals.rs`

Web (`crates/coclai_web/src`):
- `lib.rs` -> `lib.rs(파사드)` + `session_service.rs` + `turn_service.rs` + `approval_service.rs`
- 유지: `adapter.rs`, `routing.rs`, `state.rs`, `wire.rs`, `tests.rs`

Artifact (`crates/coclai_artifact/src`):
- `lib.rs` -> `lib.rs(파사드 + 도메인 모델)` + `orchestrator.rs`
- 유지: `adapter.rs`, `patch.rs`, `store.rs`, `task.rs`, `tests.rs`

Coclai Facade (`crates/coclai/src`):
- 유지: `lib.rs`, `ergonomic.rs`, `appserver.rs` (시그니처 불변 원칙)

## Execution Phases

### Phase B0. Freeze & Baseline
- 목표: 재작성 기준선 고정
- 작업:
  - 공개 API/문서/테스트 baseline snapshot 생성
  - 스키마/매니페스트/문서 동기화 상태 고정
- TASK-ID: `BB-001`, `BB-002`, `BB-003`, `BB-021`
- 검증:
  - `cargo test --workspace`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `./scripts/release_preflight.sh`
  - `./scripts/check_doc_contract_sync.sh`
  - `./scripts/check_schema_manifest.sh`
  - `COCLAI_SCHEMA_DRIFT_MODE=hard COCLAI_SCHEMA_DRIFT_SOURCE=codex ./scripts/check_schema_drift.sh`
  - `rg -n "check_product_hygiene.sh|check_doc_contract_sync.sh|check_schema_drift.sh" .github/workflows/ci.yml`

### Phase B1. Big-Bang Architecture Spec Lock
- 목표: 재작성 대상 파일/모듈 경계를 확정
- 작업:
  - Runtime/Web/Artifact 목표 모듈 구조도 확정
  - 공개 API 호환 규칙 및 금지사항 확정
- TASK-ID: `BB-004`, `BB-005`, `BB-006`
- 검증:
  - 문서 내 Target File Cut Map, 금지사항, 롤백 규칙 존재 확인
  - `rg -n "## Target File Cut Map|## Constraints|## Risk/Rollback" Docs/IMPLEMENTATION-PLAN.md`

### Phase B2. Runtime Rewrite (In Branch)
- 목표: 런타임 코어 재작성 완료
- 작업:
  - API/Runtime/State 구조 재편
  - contract/rpc/state reducer 동등 동작 보장
- TASK-ID: `BB-007`, `BB-008`, `BB-009`
- 검증:
  - `cargo test -p coclai_runtime --lib --tests`
  - `cargo test -p coclai_runtime --test contract_deterministic`
  - `cargo test -p coclai_runtime --test classify_fixtures`
  - `rg -n "coclai_web|coclai_artifact" crates/coclai_runtime/src` 결과 0건

### Phase B3. Adapter Rewrite (Web + Artifact)
- 목표: web/artifact 재작성 완료
- 작업:
  - web 세션/턴/approval 라우팅 재편
  - artifact manager/orchestrator/store 경계 재편
- TASK-ID: `BB-010`, `BB-011`, `BB-012`
- 검증:
  - `cargo test -p coclai_web --lib`
  - `cargo test -p coclai_artifact --lib`
  - cross-tenant/approval ownership, conflict/revision 관련 테스트 케이스 전부 pass

### Phase B4. Facade Compatibility Rebind
- 목표: 초보자/고급 사용자 진입점 동작 보존
- 작업:
  - `coclai` 파사드 리바인딩
  - 예제/문서/API 매핑 업데이트
- TASK-ID: `BB-013`, `BB-014`
- 검증:
  - `cargo test --workspace`
  - API snapshot diff 0 (`README.md`, `Docs/CORE_API.md`, facade exports 비교)
  - `./scripts/check_doc_contract_sync.sh`

### Phase B5. System Verification & Cutover
- 목표: 통합 검증 통과 후 단일 컷오버
- 작업:
  - workspace 통합 테스트 + 품질 게이트 + preflight
  - 성능/안정성 회귀 점검
  - 롤백 패키지 준비 후 컷오버
- TASK-ID: `BB-015`, `BB-016`, `BB-017`, `BB-018`, `BB-019`, `BB-020`
- 검증:
  - `./scripts/release_preflight.sh`
  - `./scripts/run_micro_bench.sh`
  - `./scripts/run_nightly_opt_in_gate.sh`
  - `rg -n "todo!\\(|unimplemented!\\(" crates` 결과 0건
  - `rg -n "TODO" crates --glob '!**/tests/fixtures/**'` 결과 0건

## Verification Strategy

1. 컴파일/테스트
- `cargo test --workspace`
- `cargo test -p coclai_runtime --test contract_deterministic`
- `cargo test -p coclai_runtime --test classify_fixtures`
- `cargo test -p coclai_web --lib`
- `cargo test -p coclai_artifact --lib`

2. 품질 게이트
- `cargo clippy --workspace --all-targets -- -D warnings`
- `./scripts/check_product_hygiene.sh`
- `./scripts/check_doc_contract_sync.sh`

3. 계약/스키마 게이트
- `./scripts/check_schema_manifest.sh`
- `COCLAI_SCHEMA_DRIFT_MODE=hard COCLAI_SCHEMA_DRIFT_SOURCE=codex ./scripts/check_schema_drift.sh`

4. 운영/성능
- `./scripts/run_micro_bench.sh`
- `./scripts/run_nightly_opt_in_gate.sh`
- `./scripts/release_preflight.sh`

5. 미구현/임시코드 차단
- `rg -n "todo!\\(|unimplemented!\\(" crates`
- `rg -n "TODO" crates --glob '!**/tests/fixtures/**'`
- 코드 영역에서 발견 시 컷오버 차단

## Risk/Rollback

Risk 1. 런타임 재작성 후 계약 미세 불일치 발생
- 완화: contract deterministic + fixture 분류 테스트를 컷오버 필수 게이트로 고정
- rollback: cutover 전 태그로 즉시 복귀

Risk 2. web ownership 규칙 회귀로 권한 누수
- 완화: cross-tenant/session 차단 테스트를 P0 게이트로 고정
- rollback: 컷오버 중단 + 재작성 브랜치 롤백

Risk 3. artifact 저장 일관성(revision/conflict) 깨짐
- 완화: conflict/lock/revision 회귀 테스트 선행
- rollback: 이전 릴리즈 브랜치로 즉시 리스토어

Risk 4. 빅뱅 병합 직후 운영 장애
- 완화: 릴리즈 전 smoke + nightly + preflight 동시 통과 필요
- rollback: hotfix 이전에 전체 리버트 가능하도록 단일 병합 커밋 유지

## Priority Matrix (Eisenhower)

Urgent + Important:
- `BB-001`, `BB-002`, `BB-004`, `BB-007`, `BB-010`, `BB-015`, `BB-017`, `BB-019`, `BB-020`, `BB-021`

Important, Not Urgent:
- `BB-003`, `BB-005`, `BB-006`, `BB-008`, `BB-009`, `BB-011`, `BB-012`, `BB-013`, `BB-014`, `BB-016`, `BB-018`

Urgent, Less Important:
- 없음

Neither:
- 없음

## Critical Path

`BB-001 -> BB-002 -> BB-003 -> BB-021 -> BB-004 -> BB-007 -> BB-010 -> BB-013 -> BB-015 -> BB-017 -> BB-019 -> BB-020`

## Decision Gates

- DG-B1: Baseline snapshot/계약 동기화 완료 전 코드 재작성 금지
- DG-B2: 목표 모듈 경계 승인 전 파일 이동 금지
- DG-B3: Runtime 재작성 후 contract tests 실패 시 Adapter 재작성 진입 금지
- DG-B4: Web/Artifact 보안/일관성 회귀가 0일 때만 파사드 리바인딩 허용
- DG-B5: Workspace 전체 게이트 통과 전 컷오버 금지
- DG-B6: 코드 영역(`crates/**`)에서 `todo!|unimplemented!|TODO` 탐지 0건 및 doc-contract sync pass 전 컷오버 금지

## Phase ↔ Task Traceability

- B0: `BB-001`, `BB-002`, `BB-003`, `BB-021`
- B1: `BB-004`, `BB-005`, `BB-006`
- B2: `BB-007`, `BB-008`, `BB-009`
- B3: `BB-010`, `BB-011`, `BB-012`
- B4: `BB-013`, `BB-014`
- B5: `BB-015`, `BB-016`, `BB-017`, `BB-018`, `BB-019`, `BB-020`

## Post-Cutover Hardening

- `BB-022` (P1): CI schema drift 기본 소스를 `codex`로 상향해 PR 단계 drift 검출력을 높인다.
- `BB-023` (P2): 실행/검증에 사용되지 않는 분석 레거시 문서를 정리해 문서 유지비용을 낮춘다.

## Self-Feedback Iterations

### Iteration 1 (초안 v1 -> v1.1)
- 결함 1: 단계별 좁은 검증이 부족해 실행 가능성이 낮았음.
  - 수정: 각 Phase에 구체 명령 기반 검증 항목 추가.
- 결함 2: 파일 단위 컷오버 설계가 부족해 구현 착수 포인트가 모호했음.
  - 수정: `Target File Cut Map` 추가.
- 결함 3: 미구현 코드 차단 게이트가 검증 전략에 명시되지 않았음.
  - 수정: `TODO/unimplemented` 탐지 게이트 추가.

### Iteration 2 (v1.1 -> v1.2)
- 결함 1: B5 traceability에 미구현 차단/정합성 태스크 매핑 누락.
  - 수정: `BB-019`, `BB-020`을 B5 매핑에 추가.
- 결함 2: facade 호환성 검증 기준이 실행 단계에 충분히 구체화되지 않았음.
  - 수정: B4 검증에 API snapshot diff 0 기준을 명시.
- 결함 3: 문서 정합성 점검 항목이 품질 게이트와 분리되어 추적성이 약했음.
  - 수정: 문서/계약 동기화를 B0/B4/B5에 중복 고정.

### Iteration 3 (v1.2 -> v1.3)
- 결함 1: Phase B5 TASK-ID와 Traceability 표가 불일치했음.
  - 수정: B5 TASK-ID에 `BB-019`, `BB-020`을 명시적으로 추가.
- 결함 2: Critical Path/우선순위 매트릭스가 최종 정합성 게이트를 일부 누락했음.
  - 수정: 경로/우선순위에 `BB-019`, `BB-020`을 추가.
- 결함 3: Decision Gate에 no-stub/doc-sync 하드 차단 조건이 누락됐음.
  - 수정: DG-B6 추가로 컷오버 차단 규칙 강화.

### Iteration 4 (v1.3 -> v1.4)
- 결함 1: B0에 schema drift 하드게이트가 명시되지 않아 baseline 진입 기준이 느슨했음.
  - 수정: B0 검증에 `check_schema_drift(hard,codex)` 추가.
- 결함 2: no-stub 검사 범위가 Docs까지 포함되어 컷오버 판정이 비결정적이었음.
  - 수정: 코드 영역(`crates/**`)으로 검사 대상을 한정하고 규칙 문구를 고정.
- 결함 3: Scope에 포함된 CI 워크플로우 변경 책임이 TASK에 매핑되지 않았음.
  - 수정: `BB-021` 추가 및 B0 traceability/critical path에 반영.
