# TASKS

| TASK-ID | Priority | Status(TODO\|DOING\|DONE\|BLOCKED) | Description | Done Criteria | Evidence |
|---|---|---|---|---|---|
| T-000 | P0 | DONE | 분석 착수용 계획/태스크 아티팩트 생성 | `Docs/IMPLEMENTATION-PLAN.md`, `Docs/TASKS.md`에 필수 섹션과 TASK-ID 매핑이 존재 | `Docs/IMPLEMENTATION-PLAN.md`, `Docs/TASKS.md` |
| T-010 | P0 | DONE | `coclai` 분석 코퍼스 인벤토리 확정(문서/스크립트/SCHEMAS/크레이트/테스트) | Docs/scripts/crates/SCHEMAS/tests 인벤토리와 LOC/테스트 밀도 수치가 문서화됨 | `Docs/analysis/EVIDENCE-MAP.md#phase-0-baseline-evidence` |
| T-011 | P0 | DONE | 워크스페이스 의존성 그래프/경계 맵 작성 | 내부 의존 edge 표, DAG, 역의존 금지 규칙 및 증거 명령이 문서화됨 | `Docs/analysis/EVIDENCE-MAP.md#phase-0-baseline-evidence` |
| T-012 | P0 | DONE | 업스트림 app-server 메서드 인벤토리 스냅샷 작성 | upstream 기준 커밋/파일/추출 명령과 42개 메서드 목록 + wrapper gap(32개)이 기록됨 | `Docs/analysis/EVIDENCE-MAP.md#phase-0-baseline-evidence` |
| T-015 | P0 | DONE | 레거시/불용 preflight 정리(`docs/...` 경로 혼재 및 삭제 후보 점검) | 문서 경로를 `Docs/...`로 canonicalize하고, 삭제/보류 후보와 근거를 분석 문서로 기록 | `Docs/analysis/EVIDENCE-MAP.md#legacy-prune-evidence` |
| T-020 | P0 | DONE | `README.md`의 계약 문장 분해(기능, 보장, 제약, 예외) | README 계약 문장이 `CON-001..021`로 정규화되고 타입(Function/Guarantee/Constraint/Exception) 분류가 완료됨 | `Docs/analysis/EVIDENCE-MAP.md#phase-1-contract-evidence` |
| T-021 | P1 | DONE | `Docs/*`(ARCHITECTURE/CORE_API/SCHEMA_AND_CONTRACT/SECURITY) 분해 | 문서별 주장/규칙/가정이 독립 표로 정리 | `Docs/analysis/EVIDENCE-MAP.md#phase-1-contract-evidence` |
| T-022 | P0 | DONE | 문서 선언 vs 코드 구현 일치성 매트릭스 작성 | 모든 선언 항목이 `match/mismatch/uncertain`로 분류 | `Docs/analysis/CONTRACT-MATRIX.md#declaration-vs-implementation-matrix-t-022-deliverable` |
| T-030 | P0 | DONE | `runtime` 라이프사이클/스레드 모델 시퀀스 맵 작성 | spawn/handshake/dispatch/shutdown 경로가 시퀀스로 재현 가능 | `Docs/analysis/EVIDENCE-MAP.md#phase-2-runtime-evidence` |
| T-031 | P1 | DONE | `api.rs` 책임 분해 후보 도출(`models/ops/flow`) | 분해 후보별 공개 API 영향 0 여부가 명시 | `Docs/analysis/EVIDENCE-MAP.md#phase-2-runtime-evidence` |
| T-032 | P1 | DONE | `client.rs`의 config/session/profile/hook 흐름 분석 | 중복/결합 포인트와 책임 경계 개선안이 도출 | `Docs/analysis/EVIDENCE-MAP.md#phase-2-runtime-evidence` |
| T-033 | P1 | DONE | `rpc.rs`/`rpc_contract.rs`/`schema.rs` 계약 계층 분석 | 요청/응답 검증 범위와 누락/과잉 검증 지점 분류 | `Docs/analysis/EVIDENCE-MAP.md#phase-2-runtime-evidence` |
| T-034 | P1 | DONE | `state.rs`/`metrics.rs` 성능 핫패스 및 상태 투영 한계 분석 | O(1)/O(n) 구간과 임계치/병목 가설이 정량화 | `Docs/analysis/EVIDENCE-MAP.md#phase-2-runtime-evidence` |
| T-040 | P0 | DONE | `AppServer` 파사드 메서드 커버리지 갭 분석(업스트림 대비) | 지원/미지원/의도적 미지원이 메서드 단위로 구분 | `Docs/analysis/EVIDENCE-MAP.md#phase-3-usability-evidence` |
| T-041 | P1 | DONE | `Workflow`/`quick_run` 사용성 마찰 분석(초보자 vs 전문가 경로) | 진입 비용/가시성/오류 복구성 지표가 비교됨 | `Docs/analysis/EVIDENCE-MAP.md#phase-3-usability-evidence` |
| T-042 | P1 | DONE | 에러 모델(`ClientError`/`RpcError`/`RuntimeError`) 일관성 분석 | 사용자 관점의 에러 분류 체계와 매핑안 제시 | `Docs/analysis/EVIDENCE-MAP.md#phase-3-usability-evidence` |
| T-050 | P1 | DONE | `coclai_artifact`의 patch/store/task 안전성 분석 | revision/lock/conflict 경로의 안전성·회복성 검증 | `Docs/analysis/EVIDENCE-MAP.md#phase-4-adapter-evidence` |
| T-051 | P1 | DONE | `coclai_web`의 tenant/session/approval 경계 분석 | 교차 tenant 누수 방지 규칙과 테스트 근거가 정리 | `Docs/analysis/EVIDENCE-MAP.md#phase-4-adapter-evidence` |
| T-052 | P1 | DONE | `plugin_core` 계약 버전 호환성/실패 처리 분석 | major mismatch 처리 일관성 및 파급 경로 명시 | `Docs/analysis/EVIDENCE-MAP.md#phase-4-adapter-evidence` |
| T-060 | P1 | DONE | 테스트 커버리지 히트맵 및 리스크 공백 분석 | 핵심 경로별 테스트 존재 여부/강도 레벨링 완료 | `Docs/analysis/EVIDENCE-MAP.md#phase-5-quality-evidence` |
| T-061 | P1 | DONE | 마이크로벤치 게이트의 안정성(노이즈/재시도 정책) 분석 | 회귀 탐지 신뢰도와 오탐/미탐 리스크가 정량화 | `Docs/analysis/EVIDENCE-MAP.md#phase-5-quality-evidence` |
| T-062 | P1 | DONE | 스키마 업데이트/manifest 파이프라인 드리프트 분석 | 업스트림 변경 감지 누락 포인트와 보강안 제시 | `Docs/analysis/EVIDENCE-MAP.md#phase-5-quality-evidence` |
| T-070 | P0 | DONE | 아키텍처 옵션 3안 이상 비교(비용/리스크/효익) | 옵션별 스코어카드와 채택/기각 근거 명시 | `Docs/analysis/EVIDENCE-MAP.md#phase-6-options-evidence` |
| T-071 | P0 | DONE | 동작 불변식 + 목표 의존성 규칙 확정 | 불변식/규칙이 검증 시나리오와 1:1 대응 | `Docs/analysis/EVIDENCE-MAP.md#phase-6-options-evidence` |
| T-072 | P0 | DONE | 원자적 리팩터 단계(2~6) 정의 + 단계별 검증/롤백 작성 | 각 단계에 verify/rollback 절차가 포함 | `Docs/analysis/EVIDENCE-MAP.md#phase-6-options-evidence` |
| T-073 | P0 | DONE | 최종 권고안(구조/기능/사용성 개선) 통합 보고서 작성 | 핵심 발견/옵션 비교/권고/즉시 실행 액션 포함 | `Docs/analysis/FINAL-ANALYSIS-REPORT.md#t-073-final-recommendation-integrated-report-done` |
| T-080 | P1 | DONE | 권고안 승인 이후 실행 백로그(착수 순서) 정리 | 승인 즉시 실행 가능한 backlog/게이트가 정리 | `Docs/analysis/NEXT-ACTIONS.md#t-080-post-approval-execution-backlog-and-gates-done` |
| T-101 | P0 | DONE | `BL-001` method catalog 단일화 구현(`appserver` 상수와 runtime known-method 소스 통합) | `rpc_methods`와 known-method 기준이 단일 소스에서 관리되고 회귀 테스트가 통과 | `Docs/analysis/NEXT-ACTIONS.md#bl-001-execution-log-t-101-done` |
| T-102 | P0 | DONE | `BL-002` `api.rs` 분할 1차 구현(`models/ops/flow` 경계 도입, 공개 re-export 보존) | 공개 API 경로 불변 + hook/session 회귀 테스트 통과 | `Docs/analysis/NEXT-ACTIONS.md#bl-002-execution-log-t-102-done` |
| T-103 | P0 | DONE | `BL-003` `client.rs` 분할 1차 구현(`config/session/profile/compat_guard` 경계 도입) | 세션 close/profile 기본값/guard 회귀 테스트 통과 + 공개 API 불변 | `Docs/analysis/NEXT-ACTIONS.md#bl-003-execution-log-t-103-done` |
| T-104 | P0 | DONE | `BL-004` runtime lifecycle/supervisor 상태머신 격리 1차 구현 | schema fail-fast + contract deterministic 회귀 테스트 통과 + 역의존 규칙 유지 | `Docs/analysis/NEXT-ACTIONS.md#bl-004-execution-log-t-104-done` |
| T-105 | P0 | DONE | `BL-005` 보안 mismatch 폐쇄(권한 상승 강제 조건, 외부 `rpc_id` 비노출) | 보안 경계 회귀 테스트 + `cargo test -q -p coclai_web --lib` 통과 | `Docs/analysis/NEXT-ACTIONS.md#bl-005-execution-log-t-105-done` |
| T-106 | P0 | DONE | `BL-006` drift gate 추가(`scripts/check_schema_drift.sh`, preflight 연결) | drift 주입 fail + 정상 상태 pass + preflight 연동 확인 | `Docs/analysis/NEXT-ACTIONS.md#bl-006-execution-log-t-106-done` |
| T-107 | P1 | DONE | `BL-007` 문서-코드 계약 동기화 체크 자동화 | 선언-구현 링크 커버리지 100% + `cargo check -q --workspace` 통과 | `Docs/analysis/NEXT-ACTIONS.md#bl-007-execution-log-t-107-done` |
| T-108 | P1 | DONE | `BL-008` opt-in real CLI contract 및 script 테스트 공백 보강 | nightly/opt-in 게이트 실행 로그 + 스크립트 smoke harness 통과 | `Docs/analysis/NEXT-ACTIONS.md#bl-008-execution-log-t-108-done` |
| T-109 | P0 | DONE | `BL-009` workflow 예제를 safe default + privileged opt-in 2경로로 분리 | `workflow`는 기본 실행 성공, `workflow_privileged`는 explicit escalation 경로 제공 | `Docs/analysis/NEXT-ACTIONS.md#bl-009-execution-log-t-109t-111-done` |
| T-110 | P0 | DONE | `BL-009` release preflight의 schema drift를 hard-fail로 상향 | `release_preflight.sh`에서 drift가 hard 모드로 실행되고 drift 존재 시 non-zero 종료 | `Docs/analysis/NEXT-ACTIONS.md#bl-009-execution-log-t-109t-111-done` |
| T-111 | P0 | DONE | `BL-009` rpc_direct 예제에서 turn 완료 대기 및 최종 텍스트 수집 구현 | `rpc_direct`가 `turn/completed`까지 수집 후 assistant 텍스트를 출력 | `Docs/analysis/NEXT-ACTIONS.md#bl-009-execution-log-t-109t-111-done` |
| T-112 | P0 | DONE | `BL-010` active SCHEMAS를 최신 generator 기준으로 재생성/동기화하고 불용 파일 후보를 정리 | `check_schema_drift(hard,codex)` 통과 + `check_schema_manifest.sh` 통과 + active-only 잔여 파일 없음 | `Docs/analysis/NEXT-ACTIONS.md#bl-010-execution-log-t-112-done` |
| T-113 | P0 | DONE | `BL-011` release 문서/게이트 정합성 복구(`CONTRACT-MATRIX` stale mismatch 해소 + preflight doc-sync strict 모드 고정) | `SEC-004/SEC-007`가 matrix에서 `match`로 반영 + `check_doc_contract_sync` strict mismatch gate 통과 + `release_preflight.sh` 통과 | `Docs/analysis/NEXT-ACTIONS.md#bl-011-execution-log-t-113-done` |
