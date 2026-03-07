# IMPLEMENTATION-PLAN

## Plan Metadata
- Plan ID: `C-RW-065`
- Supersedes: `C-RW-040`
- Date: `2026-03-05`
- Strategy: `Option C (전면 품질 정리, big-bang)`
- Architecture Principle: `Single Path + Data First + Functional Core / Imperative Shell`

## Status Snapshot (`2026-03-07`)
- 완료: `C-RW-072`(pure reducer/timeout/error projection 모듈 추출), `C-RW-079`(flaky/환경의존 테스트 안정화), `C-RW-080`(트리 pruning), `C-RW-081`(문서 동기화), `C-RW-082`(최종 릴리즈 게이트 통과), `C-RW-083`(opt-in 실서버 절차 문서화), `C-RW-084`(릴리즈 산출물 완결성 정리), `C-RW-085`(오류 표면 민감정보 노출 축소), `C-RW-086`(non-unix artifact lock fallback 보수화), `C-RW-087`(loaded-thread session 경로 복구), `C-RW-088`(실서버 시나리오 게이트 확장 + 릴리즈 프리플라이트 통과), `C-RW-089`(prompt_run hook scaffold 중복 제거), `C-RW-090`(실서버 approval gate 추가 + live capability boundary 명시).
- 진행 중: 없음.
- 남은 계획 태스크: 없음.
- 현재 코드 기준(`crates/coclai/src`, `*.rs`): 99 files.

## Fixed Constraints
1. 단일 경로로 정리한다. 동일 기능의 이중 경로를 유지하지 않는다.
2. `deprecated`/호환 레이어는 두지 않는다. big-bang으로 일괄 전환한다.
3. 실서버 테스트는 기본 파이프라인에서 제외하고 opt-in만 허용한다.
4. 본 크레이트는 "Codex appserver wrapper" 역할에 집중한다. 부가 기능 확장은 범위 밖이다.

## Problem Statement
- 초기 기준선에서 `crates/coclai/src`는 파일 수(94) 대비 역할 중복이 존재했다.
- 동일 개념(특히 ID 파싱, 이벤트 정규화, turn 실행 흐름, 오류 매핑)이 여러 모듈로 분산되어 개념적 노이즈가 크다.
- 최종 품질 검토에서 릴리즈 블로커 2건이 확인되었다.
  - `runtime/api/prompt_run.rs`: lag fallback 경로에서 caller timeout 상한 초과 가능
  - `runtime/core/rpc_io.rs`: in-flight call 취소 시 pending cleanup 누락 가능

## Goal
1. 래퍼 아키텍처를 최소 경로로 재구성해 “읽으면 바로 이해되는 구조”로 만든다.
2. 핵심 로직을 순수 함수(정규화/검증/리듀서/계획)로 이동하고, 부수효과(transport/fs/clock/rpc)는 얇은 shell에 격리한다.
3. 블로커를 우선 폐쇄하고, 테스트를 설계 의도 중심으로 재정렬한다.
4. 최종적으로 단순성, 정확성, 검증가능성을 동시에 충족한다.

## Definition of Done
- 기능별 단일 경로가 확정된다(중복 파서/중복 실행 엔진 제거).
- 블로커 2건이 코드+회귀 테스트로 폐쇄된다.
- 이벤트/상태/오류 처리가 데이터 모델 기준으로 일관되게 동작한다.
- 테스트가 레이어별 목적(단위/계약/통합)로 재구성되고 불필요한 중복 테스트가 제거된다.
- `docs/IMPLEMENTATION-PLAN.md`, `docs/TASKS.md`와 실제 결과가 1:1로 동기화된다.

## Non-Goals
- 신규 제품 기능 추가
- 외부 의존성 대규모 교체
- 실서버 자동 E2E 파이프라인 도입

## Data Model (Canonical)
| Entity | Core Fields | Invariant |
|---|---|---|
| SessionKey | `tenant_id`, `artifact_id`, `thread_id` | 동일 thread 재사용 시 artifact/tenant 불일치 금지 |
| TurnKey | `thread_id`, `turn_id` | 빈 문자열/loose id 금지, canonical source만 허용 |
| ApprovalKey | `approval_id`, `session_id` | 승인 응답은 단일 소유 세션에서 1회만 허용 |
| RpcEnvelope | `id`, `method`, `params|result|error` | shape 분류는 상호배타적이어야 함 |
| RuntimeState | threads/turns/items/projections | sequence는 단조증가만 반영 |
| TurnExecutionPlan | prompt/model/attachments/timeout | timeout은 caller budget 절대 상한 준수 |
| TurnExecutionResult | text/usage/terminal/error | terminal 이전 결과를 최종으로 확정하지 않음 |

## Transformations vs Side Effects
### Pure Transformations (Functional Core)
1. Envelope shape classification
2. Canonical ID parsing/validation
3. Policy normalization (`sandboxPolicy` only)
4. State reduction (event -> next state)
5. Turn output normalization (delta/completed merge)
6. Error projection/redaction
7. Timeout budget calculation (`deadline - now`)

### Side Effects (Imperative Shell)
1. Process transport read/write
2. File sink flush
3. Clock access
4. Channel send/recv
5. RPC dispatch
6. FS artifact read/write

## Current Structure (`2026-03-06`)
```text
crates/coclai/src/
  adapters/web/*            # WebAdapter facade + state/service
  appserver/*               # AppServer facade + service
  domain/artifact/*         # Artifact domain/use-cases/store
  ergonomic/*               # quick_run/workflow high-level API
  plugin/*                  # hook/plugin contract
  runtime/*                 # core/runtime/api/client/transport/state
```

핵심 단일 경로 상태:
1. turn lifecycle: `runtime/turn_lifecycle.rs` 공용 엔진 사용.
2. ID parser: `runtime/id.rs` canonical parser 단일화.
3. runtime/api 보조 계층: `runtime/api/ops.rs` 제거 후 `thread_api.rs` + `wire.rs`로 병합.
4. 정책 경로: `sandboxPolicy` 단일 경로 유지.

## Target Structure (Directional Blueprint)
```text
crates/coclai/src/
  lib.rs
  core/
    model.rs                # canonical keys/entities
    parse.rs                # canonical parser (id/envelope)
    policy.rs               # policy normalize/validate
    reduce.rs               # pure state reducer
    turn_engine.rs          # unified turn orchestration (pure planning + shell calls)
    error.rs                # canonical error projection/redaction
  shell/
    transport.rs            # process I/O only
    rpc.rs                  # request/response send/recv bridge only
    sink.rs                 # log sink + flush policy
    clock.rs                # time source boundary
    fs_store.rs             # artifact storage boundary
  facade/
    runtime.rs              # internal runtime service
    appserver.rs            # thin appserver wrapper
    web.rs                  # thin web wrapper
  tests/
    unit/
    contract/
    integration/
```

## Critical Decisions
1. `prompt_run`과 `domain/artifact/execution`의 turn 실행 흐름은 단일 엔진으로 통합한다.
2. ID 파싱은 `single canonical parser`만 사용한다. loose fallback은 전면 제거한다.
3. known-method 검증과 wire 변환은 한 경로로 통합한다.
4. 테스트는 “목적 중심 최소 세트”로 유지한다(동일 의미 중복 금지).

## Execution Phases

### Phase 0: Baseline Lock (Analysis Freeze)
- 목적: 현재 동작/계약/블로커를 재현 가능한 기준선으로 잠근다.
- 작업:
  - 릴리즈 블로커 2건 재현 테스트를 먼저 고정
  - canonical invariants 문서화
- 완료 조건:
  - baseline command set가 재현 가능
  - 블로커 재현 테스트가 실패 상태로 먼저 고정

### Phase 1: Blocker Closure (P0)
- 목적: 릴리즈 불가 원인 제거
- 작업:
  - `prompt_run` fallback 경로에 caller remaining deadline 적용
  - `rpc_io` pending entry cancellation-safe cleanup 적용
- 완료 조건:
  - 신규 회귀 테스트 통과
  - 전체 테스트에서 블로커 재발 없음

### Phase 2: Canonical Parser & Policy Unification
- 목적: 중복 해석 경로 제거
- 작업:
  - `turn_output`/`rpc`/`web/wire` ID 파싱을 단일 모듈로 통합
  - policy normalization/validation를 단일 경로화
- 완료 조건:
  - 기존 중복 파서 삭제
  - canonical parser 기반 테스트만 유지

### Phase 3: Functional Core Extraction
- 목적: 핵심 로직 순수 함수화
- 작업:
  - state reduce/timeout calc/error projection을 pure module로 추출
  - side effect 코드는 shell 경계만 남김
- 완료 조건:
  - pure module 단위 테스트 중심 커버리지 확보

### Phase 4: Turn Engine Merge
- 목적: turn 실행 엔진 단일화
- 작업:
  - `runtime/api/prompt_run` + `domain/artifact/execution` 흐름 병합
  - 공통 lifecycle(collect/interrupt/terminal) 단일 구현
- 완료 조건:
  - 중복 엔진 제거
  - artifact/api 경로가 동일 엔진을 재사용

### Phase 5: Facade Slimming
- 목적: 래퍼 역할에 맞는 얇은 표면만 유지
- 작업:
  - web/appserver facade는 orchestration 위임만 수행
  - lifecycle/ownership 규칙은 runtime service로 집중
- 완료 조건:
  - facade 파일에서 비즈니스 규칙 제거
  - 수명주기 테스트가 runtime 중심으로 이동

### Phase 6: Test Minimalization + Reliability
- 목적: 정확도 중심 테스트 체계 완성
- 작업:
  - 중복 테스트 삭제
  - flaky/timing 민감 테스트 개선
  - opt-in 실서버 테스트 게이트 명확화
  - 테스트 레이어 트리(`unit/contract/integration`)를 `docs/TEST_TREE.md`로 고정
- 완료 조건:
  - 테스트 계층(unit/contract/integration) 목적 명확
  - CI 기본 세트가 결정적이고 빠름

### Phase 7: Tree Prune + Release Gate
- 목적: 구조 간소화 완료 및 배포 준비
- 작업:
  - 불필요 파일/모듈 삭제
  - 문서 동기화 + 최종 품질 게이트
- 완료 조건:
  - 파일 트리 단순화 결과가 문서/코드에 일치
  - 릴리즈 게이트 모두 통과

## Gate Criteria
- Gate A (P0): blocker 2건 폐쇄 전 구조 개편 착수 금지
  - Blocker regression 3종은 `./scripts/check_blocker_regressions.sh`로 항상 존재/실행 검증한다.
- Gate B (Core): parser/policy 단일화 완료 전 facade 정리 금지
- Gate C (Engine): turn engine 단일화 완료 전 테스트 정리 금지
- Gate D (Release): fmt/clippy/test/security gate 통과 전 릴리즈 금지

## Verification Commands
- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- `cargo test -p coclai runtime::api::tests::run_prompt:: -- --nocapture`
- `cargo test -p coclai runtime::core::tests::core_lifecycle:: -- --nocapture`
- `./scripts/check_blocker_regressions.sh`
- `./scripts/check_product_hygiene.sh`
- `COCLAI_REAL_SERVER_APPROVED=1 COCLAI_RELEASE_INCLUDE_REAL_SERVER=1 ./scripts/release_preflight.sh`
- `./scripts/check_security_gate.sh`

## Risk & Rollback
- Risk: big-bang 병합 중 회귀 범위 확대
  - Mitigation: phase gate + task 단위 회귀 테스트 선행
- Risk: parser 통합 시 계약 호환 오판
  - Mitigation: canonical parser golden tests 고정
- Rollback: phase 단위 커밋 경계로 역추적 가능하게 유지

## Legacy Context
- `C-RW-035` ~ `C-RW-064`는 이전 Option C 라운드에서 완료됨.
- 본 계획(`C-RW-065`)은 “구조 최소화 + 함수형 코어화”를 위한 후속 전면 정리 라운드다.
