# IMPLEMENTATION-PLAN

## Goal

`/Users/axient/repository/codex`의 app-server 문서/구현체와 현재 `coclai` 래퍼를 A→Z로 분해 분석하여, 다음 질문에 증거 기반으로 답한다.

1. 현재 설계/구조/기능이 최선인가?
2. 사용성을 더 높이는 대안 구조가 무엇인가?
3. 동작 보존(behavior-preserving) 전제에서 어떤 리팩터 순서가 가장 안전한가?

완료 기준:
- 모든 `coclai` 문서/크레이트를 빠짐없이 분해한 분석 산출물 확보
- 최소 3개 아키텍처 옵션 비교 및 1개 권고안 도출
- 행동 불변식 + 의존성 규칙 + 원자적 리팩터 단계(2~6개) 확정

## Scope

포함:
- `coclai` 전체: `README.md`, `Docs/*`, `scripts/*`, `SCHEMAS/*`, `crates/*`
- 업스트림 기준선: `/Users/axient/repository/codex/codex-rs/app-server/README.md`, `/Users/axient/repository/codex/AGENTS.md`, `/Users/axient/repository/codex/codex-rs/app-server-protocol/*`

제외(이번 단계):
- 실제 코드 리팩터 구현
- API breaking change 반영
- 배포/릴리즈 실행

## Constraints

- 계획/분석 우선: 구현 변경 전에 증거 축적과 의사결정 게이트를 통과해야 함
- 동작 보존 우선: 기존 공개 API/기본 동작은 분석 기간 중 변경하지 않음
- 불확실 결론은 `uncertain`로 표기하고 가장 저비용 검증 절차를 같이 명시
- 분석 산출물은 재현 가능한 명령/파일 증거를 포함해야 함

## Data Model

### Problem

`coclai`는 app-server 래핑 라이브러리로서 이미 런타임/도메인/웹 어댑터를 갖고 있지만, 실제 사용성 관점(메서드 커버리지, API 인지 부하, 업스트림 드리프트 대응, 모듈 경계 유지보수성)에서 최적 구조인지 아직 검증되지 않았다.

### Evidence Baseline

- 워크스페이스 구성: 5 crates (`coclai`, `coclai_runtime`, `coclai_plugin_core`, `coclai_artifact`, `coclai_web`)
  - 근거: `/Users/axient/repository/coclai/Cargo.toml`, `cargo metadata --no-deps`
- 복잡도 집중:
  - `coclai_runtime/src/api.rs` 1906 LOC
  - `coclai_runtime/src/client.rs` 1032 LOC
  - `coclai_runtime/src/runtime.rs` 664 LOC
  - 근거: `wc -l` 집계
- 현재 래퍼 공개 RPC 상수: 10개 메서드(`thread/*` 일부 + `turn/start`, `turn/interrupt`)
  - 근거: `/Users/axient/repository/coclai/crates/coclai/src/appserver.rs`
- 업스트림 app-server 메서드 표면은 더 넓음(`turn/steer`, `model/list`, `skills/list`, `config/*`, `tool/requestUserInput` 등)
  - 근거: `/Users/axient/repository/codex/codex-rs/app-server/README.md`
- 테스트 분포: 테스트 마커 약 108개, 계약/실클라이언트/soak 포함
  - 근거: `rg "#[test]|#[tokio::test]|mod tests"`

### Hypotheses (with falsification)

H1. `AppServer` 표면(10개 고정 상수 + known method validator)이 실제 사용성 병목이다.
- 반증 절차:
  1. 업스트림 메서드 인벤토리 추출
  2. `appserver.rs`/`rpc_contract.rs` 지원 목록과 diff
  3. 의도적 미지원인지(정책) vs 누락인지(결함) 분류

H2. `coclai_runtime`의 대형 파일 구조(`api.rs`, `client.rs`)가 변경 파급 범위를 키운다.
- 반증 절차:
  1. 공개 타입/함수 변경 시 영향 경로 추적
  2. 테스트 모듈 의존 분포 계량
  3. 파일 분할 가상 시나리오로 빌드/테스트 영향 비교

H3. 문서 계약(`README`/`Docs/*`)과 구현 계약(`rpc_contract`, `api`, `client`) 사이에 드리프트가 누적되어 있다.
- 반증 절차:
  1. 문서 선언(지원 메서드/보장 규칙) 추출
  2. 구현체의 실제 검사/에러 규칙과 1:1 매핑
  3. 불일치 항목을 유형별(문서 과장/구현 누락/버전차)로 분류

H4. `artifact`/`web` 어댑터는 C-lite 계약 분리에 성공했지만, 런타임 내부 타입 노출로 결합도가 여전히 높다.
- 반증 절차:
  1. 어댑터 trait에 runtime 타입 누수 여부 점검
  2. cross-crate import 경계 점검
  3. 계약 버전 불일치 처리 경로의 일관성 검증

### Options (3)

Option A: 빅뱅 재구조화(대규모 즉시 리팩터)
- 장점: 단기간 구조 통일 가능
- 단점: 리스크/회귀 범위가 너무 큼, 원인 분리 어려움

Option B: 현 구조 유지 + 문서만 보강
- 장점: 비용 낮음
- 단점: 사용성 병목과 드리프트 구조 문제를 해결하지 못함

Option C (권고): 계약-우선 단계적 분해 + 동작 보존 리팩터
- 장점: 증거 기반으로 리스크를 통제하며 개선 가능
- 단점: 초기 분석 비용이 큼

### Decision

권고안은 **Option C**.  
이유: 현재 코드베이스는 계약/테스트 자산이 이미 풍부하므로, 이를 기준선으로 삼아 단계적 분해를 수행할 때 정확도와 안전성을 동시에 확보할 수 있다.

## Behavior Invariants

아래는 리팩터 전후 반드시 유지해야 하는 불변식이다.

1. 세션 라이프사이클: `connect -> setup/run -> ask -> close -> shutdown` 호출 의미 보존
2. 기본 보안값: `approval=never`, `sandbox=readOnly` 경로 보존
3. 계약 검증 모드 기본값: `RpcValidationMode::KnownMethods` 보존
4. `Session::close()` 후 핸들 재사용 거절 동작 보존
5. 훅 실패 fail-open 정책 + `HookReport` 누적 동작 보존
6. 스키마 가드/manifest fail-fast 동작 보존

## Target Dependency Rules

목표 의존성 규칙(방향 고정):

1. `coclai` -> `coclai_runtime` (facade only)
2. `coclai_runtime` -> `coclai_plugin_core` (hook contract only)
3. `coclai_artifact` -> `coclai_runtime` + own domain modules
4. `coclai_web` -> `coclai_runtime` + own adapter/state modules
5. `coclai_runtime`는 `artifact/web`를 참조하지 않는다(역의존 금지)

## Execution Phases

### Phase 0. 기준선 고정

- 목적: 분석 입력을 고정하고 재현 가능한 인벤토리 확보
- TASK-ID: `T-010`, `T-011`, `T-012`
- 산출물: `Docs/analysis/EVIDENCE-MAP.md#phase-0-baseline-evidence`
- 좁은 검증: 파일/LOC/의존성/메서드 목록이 명령 결과와 일치

### Phase 1. 문서 분해 분석

- 목적: README + docs 계약 문장을 구조화 데이터로 변환
- TASK-ID: `T-020`, `T-021`, `T-022`
- 산출물:
  - `Docs/analysis/CONTRACT-MATRIX.md`
  - `Docs/analysis/EVIDENCE-MAP.md#phase-1-contract-evidence`
- 좁은 검증: 문서 선언 항목별 구현 참조 경로 1개 이상 연결

### Phase 2. 런타임 코어 분해 분석

- 목적: `runtime/client/api/rpc/state`의 책임·경계·복잡도 맵 작성
- TASK-ID: `T-030`, `T-031`, `T-032`, `T-033`, `T-034`
- 산출물: `Docs/analysis/EVIDENCE-MAP.md#phase-2-runtime-evidence`
- 좁은 검증: 모듈별 입력/출력/부수효과 표가 코드와 일치

### Phase 3. 파사드/사용성 분해 분석

- 목적: `AppServer`, `Workflow`, `quick_run` 사용성 병목과 누락 경로 식별
- TASK-ID: `T-040`, `T-041`, `T-042`
- 산출물: `Docs/analysis/EVIDENCE-MAP.md#phase-3-usability-evidence`
- 좁은 검증: 업스트림 메서드 대비 지원 갭 테이블 완성

### Phase 4. 어댑터/경계 분석

- 목적: `artifact/web/plugin_core` 계약 분리 수준과 안전성 점검
- TASK-ID: `T-050`, `T-051`, `T-052`
- 산출물: `Docs/analysis/EVIDENCE-MAP.md#phase-4-adapter-evidence`
- 좁은 검증: tenant/approval/contract mismatch 경로 재현 시나리오 작성

### Phase 5. 품질 게이트 분석

- 목적: 테스트/성능/스키마 파이프라인의 신뢰도와 공백 확인
- TASK-ID: `T-060`, `T-061`, `T-062`
- 산출물: `Docs/analysis/EVIDENCE-MAP.md#phase-5-quality-evidence`
- 좁은 검증: 각 게이트의 실패 시나리오와 탐지 가능성 매핑

### Phase 6. 대안 비교 + 리팩터 설계

- 목적: 구조 개선 옵션 비교 후 원자적 단계 설계
- TASK-ID: `T-070`, `T-071`, `T-072`
- 산출물: `Docs/analysis/EVIDENCE-MAP.md#phase-6-options-evidence`
- 좁은 검증: 옵션별 비용/리스크/가시성 점수 + 선택 근거 명시

### Phase 7. 최종 권고안 확정

- 목적: A→Z 분석 결과를 최종 권고안과 실행백로그로 수렴
- TASK-ID: `T-073`, `T-080`
- 산출물:
  - `Docs/analysis/FINAL-ANALYSIS-REPORT.md`
  - `Docs/analysis/NEXT-ACTIONS.md`
- 좁은 검증: 모든 TASK-ID가 근거 파일과 1:1 연결

### Phase 8. 외부 공개 릴리즈 하드닝

- 목적: 외부 공개 전에 실패율/보안/계약 가시성을 높이는 최소 수정 적용
- TASK-ID: `T-109`, `T-110`, `T-111`
- 산출물:
  - `crates/coclai/examples/workflow.rs`
  - `crates/coclai/examples/workflow_privileged.rs`
  - `crates/coclai/examples/rpc_direct.rs`
  - `scripts/release_preflight.sh`
  - `README.md`, `Docs/CORE_API.md`, `Docs/SCHEMA_AND_CONTRACT.md`
- 좁은 검증:
  1. `workflow` safe default가 추가 플래그 없이 실행됨
  2. `workflow_privileged`는 explicit opt-in(`allow_privileged_escalation`) 경로를 제공함
  3. `rpc_direct`가 `turn/completed`까지 대기 후 최종 텍스트를 출력함
  4. preflight의 schema drift gate가 `hard` 모드에서 실패를 차단함

## Atomic Refactor Steps (Behavior-Preserving, 2~6)

아래 6개는 **구현 승인 후** 수행할 원자 단계 제안이다.

1. RPC 메서드 카탈로그 단일화(`appserver.rs` 상수 + `rpc_contract.rs` known-method 동기화)
- 검증: 기존 10개 메서드 회귀 테스트 + 누락/중복 diff 0
- 롤백: 기존 수동 상수/검증 로직으로 즉시 복원

2. `api.rs` 분할(`models`, `thread_ops`, `turn_ops`, `run_flow`)
- 검증: 공개 re-export 경로/타입 시그니처 불변
- 롤백: 모듈 분할 revert(인터페이스 변경 없음)

3. `client.rs` 분할(`config`, `session`, `compat_guard`, `run_profile`)
- 검증: `Client::*` 공개 API snapshot 일치
- 롤백: 파일 병합 revert

4. 런타임 핸드셰이크/재시작 상태머신 격리(`runtime/lifecycle`, `runtime/supervisor`)
- 검증: handshaking/restart 실패 경로 테스트 유지
- 롤백: 기존 진입점 경로로 재연결

5. 업스트림 스키마/메서드 드리프트 자동 감지 스크립트 추가
- 검증: 샘플 드리프트 주입 시 CI 실패
- 롤백: 드리프트 체크를 non-blocking 모드로 전환

6. 문서-코드 계약 동기화 파이프라인 확립(`README`/`Docs/*` 자동 체크리스트)
- 검증: 체크리스트에서 선언-구현 링크 100% 채움
- 롤백: 수동 검토 플로우로 임시 복귀

## Verification Strategy

분석 단계별 공통 검증:

1. 증거 링크 검증: 각 결론마다 최소 1개 파일 근거 또는 재현 절차 포함
2. 재현 명령 검증: 인벤토리/갭 분석 명령이 동일 결과를 재현
3. 계약 검증:
   - `cargo test -p coclai_runtime --test contract_deterministic`
   - `cargo test -p coclai_runtime --test classify_fixtures`
   - `./scripts/check_schema_manifest.sh`
4. 구조 회귀 검증(구현 단계에서):
   - `cargo test -p coclai --lib`
   - `cargo test -p coclai_runtime --lib`
   - `cargo test -p coclai_artifact --lib`
   - `cargo test -p coclai_web --lib`
5. 성능 게이트:
   - `./scripts/run_micro_bench.sh`
6. 선택적 실CLI 스모크:
   - `APP_SERVER_CONTRACT=1 cargo test -p coclai_runtime --test contract_real_cli -- --nocapture`

## Risk/Rollback

주요 리스크:

1. 업스트림 app-server 버전 변화로 분석 결과 조기 노후화
- 완화: 분석 보고서에 스키마/문서 기준 시점 명시
- 롤백: 기준 버전 고정 후 재분석

2. 과도한 파일 분할로 탐색성이 오히려 하락
- 완화: 단계별 분할 + API 표면 불변 검증
- 롤백: 분할 단계 단위 revert

3. 사용성 개선 시도 중 계약 검증 약화
- 완화: `KnownMethods` 기본값과 스키마 가드 유지
- 롤백: strict validation 경로 강제 복귀

4. 웹/도메인 어댑터에서 권한/tenant 경계 누수
- 완화: approval/session ownership 테스트 우선 강화
- 롤백: adapter 기능 플래그로 노출 축소

## Priority Matrix (Eisenhower)

### Urgent + Important
- `T-010`, `T-011`, `T-012`, `T-022`, `T-040`, `T-070`

### Important, Not Urgent
- `T-031`, `T-032`, `T-033`, `T-034`, `T-041`, `T-071`, `T-072`

### Urgent, Less Important
- `T-060`, `T-061`, `T-062`

### Neither
- 없음(이번 범위는 전부 구조 의사결정에 직접 기여)

## Critical Path

1. `T-010` -> `T-011` -> `T-012` (기준선 확보)
2. `T-020` -> `T-021` -> `T-022` (문서 계약 고정)
3. `T-030` -> `T-031` -> `T-033` -> `T-034` (코어 분해)
4. `T-040` + `T-041` + `T-052` (사용성/경계 검증)
5. `T-070` -> `T-071` -> `T-072` -> `T-073` (의사결정 수렴)

## Decision Gates

- DG-1 (Phase 0 종료): 인벤토리 누락 0건
- DG-2 (Phase 1 종료): 문서 선언-구현 매핑률 100%
- DG-3 (Phase 3 종료): 업스트림 대비 지원 갭 분류 완료(의도/결함)
- DG-4 (Phase 6 종료): 3개 옵션 비교 + 1개 권고안 승인
- DG-5 (Phase 7 종료): 최종 분석 보고서 + 실행 백로그 승인
