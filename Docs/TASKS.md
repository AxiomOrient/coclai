# TASKS (Big-Bang Rewrite)

| TASK-ID | Priority | Status | Description | Done Criteria | Evidence |
|---|---|---|---|---|---|
| BB-001 | P0 | DONE | 빅뱅 재작성 baseline 고정(테스트/품질/문서 상태 스냅샷) | baseline 명령 세트가 모두 실행되고 결과가 저장됨 | `cargo test --workspace` pass, `cargo clippy --workspace --all-targets -- -D warnings` pass, `bash scripts/release_preflight.sh` pass |
| BB-002 | P0 | DONE | 공개 API/문서 계약 snapshot 생성 | 재작성 전 public API 목록과 문서 계약이 스냅샷으로 고정됨 | `README.md`, `Docs/CORE_API.md`, `crates/coclai/src/lib.rs` |
| BB-003 | P1 | DONE | schema/manifest/doc-sync 상태 동결 | drift/manifest/doc-sync 체크가 전부 pass | `bash scripts/check_schema_manifest.sh` pass, `COCLAI_SCHEMA_DRIFT_MODE=hard COCLAI_SCHEMA_DRIFT_SOURCE=codex bash scripts/check_schema_drift.sh` pass, `COCLAI_DOC_SYNC_MODE=hard COCLAI_DOC_SYNC_FAIL_ON_MISMATCH=1 bash scripts/check_doc_contract_sync.sh` pass |
| BB-004 | P0 | DONE | Big-Bang 타깃 구조 명세 고정(runtime/web/artifact 파일 경계) | 각 crate의 타깃 모듈 트리와 의존 규칙이 문서화됨 | `Docs/IMPLEMENTATION-PLAN.md`의 `Target File Cut Map` 섹션 |
| BB-005 | P1 | DONE | 공개 API 호환 정책 고정(금지/허용 변경 범위) | 재작성 중 허용되는 내부 변경과 금지 breaking change가 명시됨 | `Docs/IMPLEMENTATION-PLAN.md`의 `Constraints` 섹션 |
| BB-006 | P1 | DONE | 컷오버 정책/롤백 절차 문서화 | 컷오버 중단 조건과 롤백 절차가 체크리스트로 존재 | `Docs/IMPLEMENTATION-PLAN.md`의 `Risk/Rollback` 섹션, `Docs/CUTOVER-OPERATIONS.md` |
| BB-007 | P0 | DONE | Runtime 재작성 1: runtime core/spawn/dispatch 경계 재편 | `coclai_runtime` 컴파일+핵심 테스트 통과, 역의존 0 | `cargo test -p coclai_runtime --lib --tests` pass (138/138), `rg -n "coclai_web|coclai_artifact" crates/coclai_runtime/src` => none |
| BB-008 | P1 | DONE | Runtime 재작성 2: API 흐름(thread/turn/prompt) 재편 | contract deterministic + classify fixtures 통과 | `cargo test -p coclai_runtime --test contract_deterministic` pass (3/3), `cargo test -p coclai_runtime --test classify_fixtures` pass (4/4) |
| BB-009 | P1 | DONE | Runtime 재작성 3: state reducer/pruning 재편 | `state` 관련 단위/통합 테스트 통과 + projection 동등성 확인 | `cargo test -p coclai_runtime --lib --tests` pass, `state::tests::*` + `runtime::tests::state_snapshot_*` pass |
| BB-010 | P0 | DONE | Web 재작성: session/turn/approval 서비스 경계 분리 | tenant/session ownership 회귀 0 | `cargo test -p coclai_web --lib` pass (13/13), `crates/coclai_web/src/{session_service.rs,turn_service.rs,approval_service.rs}` |
| BB-011 | P1 | DONE | Web 이벤트 라우팅/SSE 경로 정합성 보강 | subscribe/post approval 관련 회귀 테스트 통과 | `cargo test -p coclai_web --lib` pass (13/13), `crates/coclai_web/src/subscription_service.rs` |
| BB-012 | P0 | DONE | Artifact 재작성: manager/orchestrator/store 경계 분리 | DocGenerate/DocEdit/conflict/lock/revision 회귀 0 | `cargo test -p coclai_artifact --lib` pass (26/26), `crates/coclai_artifact/src/orchestrator.rs` |
| BB-013 | P0 | DONE | Facade 재결선: `coclai` 공개 진입점 재바인딩 | `quick_run`/`Workflow`/`AppServer` 시그니처/동작 불변 + 런타임 진입점 정상 동작 | `cargo test -p coclai --lib --tests` pass (17/17), `crates/coclai/src/lib.rs` |
| BB-014 | P1 | DONE | 예제/문서/계약 동기화 | README/CORE_API/SCHEMA 계약 문서 mismatch 0 | `COCLAI_DOC_SYNC_MODE=hard COCLAI_DOC_SYNC_FAIL_ON_MISMATCH=1 bash scripts/check_doc_contract_sync.sh` -> mismatch 0 |
| BB-015 | P0 | DONE | 빅뱅 통합 검증 1: workspace test + clippy | workspace 테스트/정적 점검 전부 green | `cargo test --workspace` pass, `cargo clippy --workspace --all-targets -- -D warnings` pass |
| BB-016 | P1 | DONE | 빅뱅 통합 검증 2: micro bench + nightly gate | 성능 회귀 임계치 통과 + nightly opt-in 통과 | `bash scripts/run_micro_bench.sh` pass (`max-regression=15%` 이내), `bash scripts/run_nightly_opt_in_gate.sh` pass |
| BB-017 | P0 | DONE | 최종 preflight + 컷오버 리허설 | `release_preflight.sh` pass + 컷오버 체크리스트 complete | `bash scripts/release_preflight.sh` pass, `Docs/CUTOVER-OPERATIONS.md`의 rehearsal checklist |
| BB-018 | P0 | DONE | 컷오버 실행/사후 안정화 모니터링 계획 확정 | 롤백 트리거/SLO/알림 담당자/관측 지표가 문서에 완전 기재됨 | `Docs/CUTOVER-OPERATIONS.md` (rollback trigger, SLO, alert owner, monitoring matrix) |
| BB-019 | P0 | DONE | 미구현/임시코드 차단 게이트 고정 | 코드 영역(`crates/**`)에서 `TODO|todo!|unimplemented!` 탐지 결과가 컷오버 허용 기준(0건) 충족 | `rg -n "todo!\\(|unimplemented!\\(" crates` -> 0, `rg -n "TODO" crates --glob '!**/tests/fixtures/**'` -> 0 |
| BB-020 | P0 | DONE | 문서-코드-계약 정합성 최종 감사 | ARCHITECTURE/CORE_API/SCHEMA/README와 코드 참조가 상호 일치 | `COCLAI_DOC_SYNC_MODE=hard COCLAI_DOC_SYNC_FAIL_ON_MISMATCH=1 bash scripts/check_doc_contract_sync.sh` (`coverage=78/78`, `mismatch=0`) |
| BB-021 | P0 | DONE | CI 워크플로우 정합성 고정: scope에 선언된 `.github/workflows/ci.yml` 게이트 경로 검증 | CI 워크플로우에 품질/계약 게이트 단계가 존재하고 계획 문서와 불일치 0 | `rg -n \"check_product_hygiene.sh|check_doc_contract_sync.sh|check_schema_drift.sh\" .github/workflows/ci.yml` -> lines `29,32,35` |
| BB-022 | P1 | DONE | CI schema drift 소스 기준 상향(`active` -> `codex` 기본) | PR 단계 drift 게이트가 업스트림 SoT(codex) 기준으로 실행됨 | `.github/workflows/ci.yml:35` (`COCLAI_SCHEMA_DRIFT_SOURCE=${COCLAI_CI_SCHEMA_DRIFT_SOURCE:-codex}`), `bash scripts/release_preflight.sh` pass (`schema-drift OK (mode=hard, source=codex)`) |
| BB-023 | P2 | DONE | 구현 완료된 분석/레거시 문서 정리 | 실행에 사용되지 않는 구 분석 문서 제거 + 참조 무결성 유지 | 삭제: `Docs/analysis/{NEXT-ACTIONS.md,FINAL-ANALYSIS-REPORT.md,EVIDENCE-MAP.md,BIGBANG-BASELINE-SNAPSHOT-2026-02-28.md,BIGBANG-FILE-AUDIT-2026-02-28.md}`, `rg -n "NEXT-ACTIONS\\.md|FINAL-ANALYSIS-REPORT\\.md|EVIDENCE-MAP\\.md|BIGBANG-BASELINE-SNAPSHOT-2026-02-28\\.md|BIGBANG-FILE-AUDIT-2026-02-28\\.md" README.md Docs .github scripts crates -g '!Docs/TASKS.md'` -> 0 |
