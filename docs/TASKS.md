# TASKS

| TASK-ID | Priority | Status | Description | Done Criteria | Evidence |
|---|---|---|---|---|---|
| C-RW-035 | P0 | DONE | 크레이트 이름 변경 검토 및 파급 분석 | rename 유지/보류 결정과 근거가 명시된다 | `docs/IMPLEMENTATION-PLAN.md` Decision 섹션 |
| C-RW-036 | P0 | DONE | 패키지명 하드코딩 제거(리네임 준비) | 스크립트에서 패키지명을 env 기반으로 주입 가능해야 한다 | `scripts/release_preflight.sh`, `scripts/check_security_gate.sh` (`COCLAI_PKG`) |
| C-RW-037 | P1 | DONE | 스크립트/스키마 노이즈 추가 정리 | 중복 문서 트리/벤치 바이너리 경로가 제거되고 스키마 강제관리 코드가 없어야 한다 | 삭제: `docs/` 중복 경로, `crates/coclai/src/runtime/bin/`, `crates/coclai/Cargo.toml`의 `[[bin]]`, 검색: `rg -n "SCHEMA|schema manifest|schema drift|manage_schemas"` |
| C-RW-038 | P0 | DONE | 최종 검증 | 포맷/핵심 테스트/스크립트 문법 검증 통과 | `cargo fmt --all --check`, `cargo test -p coclai runtime::{client,core,api}::tests:: -- --nocapture`, `bash -n scripts/*.sh` |
| C-RW-039 | P1 | DONE | 테스트/임시 경로의 제품명 하드코딩 축소 | `src` 내부 `coclai_` 접두 문자열이 제거되거나 정책적 예외만 남아야 한다 | 변경: `ergonomic/tests/unit.rs`, `runtime/client/tests.rs`, `domain/artifact/tests/{runtime_tasks.rs,unit_core.rs}`, `runtime/sink.rs`; 검증: `rg -n "coclai_" crates/coclai/src` (0건) |
