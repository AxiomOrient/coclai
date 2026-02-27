# Final Analysis Report

## Scope

- Completed in this step: **T-073**
- In scope artifacts:
  - `/Users/axient/repository/coclai/Docs/analysis/CONTRACT-MATRIX.md`
  - `/Users/axient/repository/coclai/Docs/analysis/EVIDENCE-MAP.md`
  - `/Users/axient/repository/coclai/Docs/analysis/NEXT-ACTIONS.md`
  - `/Users/axient/repository/coclai/Docs/IMPLEMENTATION-PLAN.md`
  - `/Users/axient/repository/coclai/Docs/TASKS.md`
- Out of scope in this step:
  - 실제 코드 리팩터 구현/배포 (`T-080` 이후 실행 백로그 단계)

## Snapshot Metadata

- Collected at (UTC): `2026-02-27T08:38:07Z`
- Baseline repo/branch: `coclai@dev`
- Source version: working tree dirty (analysis/docs updates in progress)
- Upstream baseline reference:
  - `/Users/axient/repository/codex/codex-rs/app-server-protocol/schema/json/ClientRequest.json`
  - `/Users/axient/repository/codex/codex-rs/app-server/README.md`

## Verification Map (Narrow -> Broader)

1. Narrow: Phase 0~6의 정량 지표와 판정 결과를 단일 모델로 통합한다.
2. Medium: 구조/기능/사용성/품질 게이트 관점에서 `현재 설계가 최선인지`를 yes/no와 근거로 분해한다.
3. Broader: 옵션 비교(Option A/B/C), 최종 권고안, 즉시 실행 액션을 한 문서에서 연결한다.
4. Cross-check: `T-071` 불변식/의존성 규칙과 `T-072` 원자 단계가 권고안에 직접 매핑되는지 확인한다.

## T-073 Final Recommendation Integrated Report (DONE)

### A. Data Model (A->Z 통합 축)

최종 평가는 아래 6축으로 고정한다.

1. `ContractIntegrity`
   - 문서 선언과 구현의 합치 수준 (`match/mismatch/uncertain`)
2. `StructuralMaintainability`
   - 모듈 경계/복잡도/의존성 방향성의 유지보수 용이성
3. `FacadeUsability`
   - 래퍼 표면의 인지부하, 가시성, 에러 회복성
4. `BoundarySafety`
   - artifact/web/plugin_core 경계의 tenant/contract/충돌 안전성
5. `QualityGateReliability`
   - 테스트/벤치/schema pipeline이 회귀를 안정적으로 잡는지
6. `RefactorExecutability`
   - 불변식 보존 하에 단계적 리팩터를 안전하게 수행할 수 있는지

### B. Executive Verdict

질문 1. 현재 설계/구조/기능이 최선인가?
- 결론: **아니다(부분 최적)**.
- 근거:
  1. 핵심 런타임/경계 안전성은 이미 강하다.
  2. 그러나 facade 가시성(`10 first-class vs 32 pass-through`), schema drift gate, 일부 보안 계약 불일치가 남아 있다.
  3. 대형 파일 hotspot(`api.rs 1906 LOC`, `client.rs 1032 LOC`)은 구조 진화 비용을 높인다.

질문 2. 더 나은 방식이 있는가?
- 결론: **있다**. `Contract-first staged refactor (Option C)`가 비용/리스크/효익 균형에서 최적이다.

질문 3. 안전한 리팩터 순서는?
- 결론: `R1 -> R2 -> R3 -> R4 -> R5 -> R6`의 원자 단계와 단계별 verify/rollback이 가장 안전하다.

### C. Integrated Findings (구조/기능/사용성/품질)

#### C1. Contract Integrity

- 선언-구현 매트릭스: 총 `78`개 항목
  - `match=64`
  - `mismatch=2`
  - `uncertain=12`
- 핵심 mismatch:
  1. `SEC-004`: 권한 상승이 명시적 사용자 요청/범위 검증을 코드 레벨에서 강제하지 못함
  2. `SEC-007`: 외부 직렬화 경로에서 내부 `rpc_id` 노출 가능성

판단:
- 계약 기반은 성숙하지만, 보안 경계의 일부 선언은 구현 강제력이 부족하다.

#### C2. Structural Maintainability

- 워크스페이스 의존성은 목표 방향과 일치한다.
  - `coclai -> coclai_runtime`
  - `coclai_runtime -> coclai_plugin_core`
  - `coclai_artifact -> coclai_runtime`
  - `coclai_web -> coclai_runtime`
  - 금지 역의존(`runtime -> artifact|web`) 없음
- hotspot 파일 집중:
  - `api.rs` 1906 LOC
  - `client.rs` 1032 LOC
  - `runtime.rs` 664 LOC

판단:
- 현재 구조는 동작 보존에 성공했지만, 변경 파급을 줄이려면 단계적 파일 분해가 필요하다.

#### C3. Facade Functionality and Usability

- upstream client-request method set: `42`
- `AppServer` first-class constants + known validation: `10`
- 기본 경로 pass-through callable: `32`
- unsupported method: `0` (기본 경로로는 호출 가능)

해석:
- 기능 "부재"라기보다 "표면 가시성/안전등급 표시 부족" 문제다.
- 사용자 입장에서는 first-class와 pass-through 차이가 문서/타입에서 충분히 드러나지 않는다.

#### C4. Beginner vs Expert Path

- `quick_run` 계열: 1-call one-shot + 정리(shutdown) 내장
- `Workflow`: 최소 3-call + 호출자 수동 정리 책임
- 예제 LOC 비교: `workflow`가 `quick_run` 대비 약 `2.9x`

판단:
- 초보자 경로는 우수하다.
- 전문가 경로는 제어력은 높지만 회복 보일러플레이트가 크다.

#### C5. Error Model Consistency

- 동일 상태에서도 request/notify 경로가 다른 에러 타입을 노출하는 구간이 있다.
  - 예: `NotInitialized` vs `InvalidRequest("runtime is not initialized")`
  - 예: payload contract 위반 시 `RpcError::InvalidRequest` vs `RuntimeError::InvalidConfig`
- `RuntimeError::Timeout`은 선언 대비 사용 경로가 불명확한 `uncertain` 항목이다.

판단:
- 타입 자체는 충분하지만 사용자 관점 category 투영이 부족하다.

#### C6. Adapter Boundary Safety

- `coclai_artifact`: revision/lock/conflict 경계가 강하게 고정됨
- `coclai_web`: tenant/session/thread/approval ownership 경계가 일관적
- `plugin_core`: major mismatch fail-fast가 artifact/web 모두에서 일관됨

판단:
- adapter 경계는 전체적으로 견고하며, 부정 시나리오 테스트를 보강하면 신뢰도가 더 올라간다.

#### C7. Quality Gate Reliability

- test marker: 총 `203`
- 핵심 경로는 주로 `H3/H2` 강도
- 주요 공백:
  1. scripted release/schema pipeline은 `H0` (스크립트 자체 테스트 부재)
  2. real CLI contract parity는 `opt-in` (`APP_SERVER_CONTRACT=1`)
  3. schema external drift 탐지가 기본 gate에 포함되지 않음

정량 drift:
- active schema files: `85`
- generated-now files: `142`
- `missing in active=59`, `only in active=2`, `hash-diff(common)=41`

### D. Option Comparison and Final Decision

| Option | Utility | Decision | 이유 요약 |
|---|---:|---|---|
| A. Big-bang rewrite | 3.20 | Reject | 효익은 크지만 delivery cost/regression risk가 과도 |
| B. Facade-only expansion | 2.70 | Reject | 단기 사용성 개선만 가능, drift/구조 문제 미해결 |
| C. Contract-first staged refactor | 4.15 | **Adopt** | 검증 자산 재사용 가능, 리스크 통제, 구조 개선 동시 달성 |

최종 권고:
- **Option C 채택**
- `T-071` 불변식/의존성 규칙 + `T-072` 원자 단계를 실행 계약으로 고정

### E. Recommended Target State (구조/기능/사용성)

1. Method catalog single source
   - `rpc_methods`와 known-method 검증 집합 자동 동기화
2. Runtime core modularization
   - `api/client/runtime`를 원자 단계로 분할하되 공개 시그니처 불변 유지
3. Explicit 2-tier facade contract
   - `first-class`와 `pass-through`를 문서/타입에서 명확히 구분
4. Drift-aware quality gate
   - 내부 manifest 일치뿐 아니라 external parity drift를 자동 감지
5. Security contract closure
   - 권한 상승 강제 조건, 외부 식별자 노출 규칙을 코드/테스트로 고정

### F. Immediate Actions (Execution-Ready, Pre-Approval Queue)

| Action ID | Priority | Action | Verify Gate | Rollback Trigger |
|---|---|---|---|---|
| IA-01 | P0 | `R1` method catalog 단일화 (`appserver`/`rpc_contract` 동기화 자동화) | known-method 테스트 + set diff 0 | 카탈로그 생성 변경 revert |
| IA-02 | P0 | `R2` `api.rs` 분할 (`models/ops/flow`) | `run_prompt`/hook 회귀 테스트 | api 분할 커밋 revert |
| IA-03 | P0 | `R3` `client.rs` 분할 (`config/session/profile/guard`) | session close/open/profile 기본값 회귀 테스트 | client 분할 커밋 revert |
| IA-04 | P0 | `R4` lifecycle/supervisor 격리 | handshake/restart/schema fail-fast 테스트 | runtime 분할 커밋 revert |
| IA-05 | P0 | `R5` schema drift gate 추가 (`check_schema_drift.sh`) | drift 주입 시 실패 + 정상 시 통과 | soft-mode 전환 후 스크립트 revert |
| IA-06 | P1 | `R6` 문서-코드 계약 동기화 체크리스트 자동화 | 선언-구현 링크 100% + workspace check | non-blocking 리포트 모드 전환 |
| IA-07 | P0 | 보안 mismatch 폐쇄 (`SEC-004`, `SEC-007`) | 권한 상승 강제 테스트 + rpc_id 비노출 테스트 | 보안 가드 변경 revert |
| IA-08 | P1 | `T-080` 실행 백로그 문서화 (`NEXT-ACTIONS`) | 승인 즉시 착수 가능 큐/게이트 정리 | 문서 단계 재조정 |

### G. Risks, Uncertainties, Cheapest Verification

1. `uncertain`: 전문가 경로(`Workflow`)의 실패 회복 통합 테스트 밀도
   - cheapest step: run 실패 + shutdown 실패 동시 보존 통합 테스트 추가
2. `uncertain`: `RuntimeError::Timeout` dead variant 여부
   - cheapest step: 사용 경로 탐지 CI 체크 + 필요 시 deprecated 계획
3. `uncertain`: adapter contract version의 runtime 중 변경 대응 정책
   - cheapest step: mutable test adapter로 version drift 재현 테스트
4. `uncertain`: schema prune 목록의 의도적 subset 여부
   - cheapest step: 정책 파일화 + 항목별 근거 주석 추가

### H. Transformations vs Side Effects

| Step | Transformation | Side Effect | Evidence |
|---|---|---|---|
| phase integration | Phase 0~6 결과를 6축 데이터 모델로 통합 | none | 본 문서 A~G 섹션 |
| decision convergence | 옵션/불변식/원자단계를 최종 권고안으로 수렴 | none | 본 문서 D~F 섹션 |
| final verification | manifest/workspace check로 최종 무결성 확인 | 빌드/스크립트 실행 | 아래 Self-Verify 로그 |

### I. Perf Notes

- 본 `T-073`은 문서 통합 단계로 런타임 경로 변경은 없다.
- 리팩터 실행 전략의 성능 모델은 `O(k * verify_cost)` (`k = 원자 단계 수`)로, big-bang 단일 대규모 검증보다 실패 복구 비용이 작다.
- 현재 병목은 실행 성능보다 구조 변경 시 검증 비용/인지 부하에 가깝다.

### J. Deterministic Evidence Commands

```bash
# 문서-구현 핵심 지표 재확인
rg -n "Total declarations evaluated: `78`|`match`: `64`|`mismatch`: `2`|`uncertain`: `12`" Docs/analysis/CONTRACT-MATRIX.md
rg -n "Upstream method set `42`|first-class surfaced `10`|pass-through callable `32`" Docs/analysis/EVIDENCE-MAP.md
rg -n "active=85|generated-now=142|missing in active=59|hash-diff\\(common\\)=41" Docs/analysis/EVIDENCE-MAP.md
rg -n "Option utility: `A=3.20`, `B=2.70`, `C=4.15`|R1 -> R2 -> R3 -> R4 -> R5 -> R6" Docs/analysis/EVIDENCE-MAP.md

# 품질 게이트 최종 확인
./scripts/check_schema_manifest.sh
cargo check -q --workspace
```

### K. Task Sync Evidence

- Lifecycle: `T-073` `TODO -> DOING -> DONE`
- Artifact:
  - `Docs/TASKS.md`
  - `Docs/analysis/FINAL-ANALYSIS-REPORT.md#t-073-final-recommendation-integrated-report-done`

### L. Self-Verify Execution Evidence (T-073)

```bash
./scripts/check_schema_manifest.sh
# result: manifest OK

cargo check -q --workspace
# result: success (no diagnostics)
```
