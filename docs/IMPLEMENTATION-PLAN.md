# IMPLEMENTATION-PLAN

## Plan Metadata
- Plan ID: `C-RW-035`
- Scope: `crates/coclai` 이름/자산 정리
- Date: `2026-03-03`

## Goal
1. `coclai` 크레이트 이름 변경 가능성을 검토하고, 즉시 가능한 호환성 개선을 반영한다.
2. 구현에 남아있는 스크립트/스키마 관련 노이즈를 제거한다.

## Decision
- 이번 변경에서는 크레이트 이름을 유지한다 (`coclai`).
- 이유: 패키지 이름 변경은 downstream 의존(`Cargo.toml`, `use coclai::...`, CI/스크립트) 전반의 호환성 파급이 크다.
- 대신 즉시 개선으로 패키지명 하드코딩을 스크립트에서 제거해 rename 준비도를 높인다.

## Steps
1. 패키지명 하드코딩 제거 (`scripts/*` -> `COCLAI_PKG` 지원)
2. 문서 경로 단일화 (`docs/`)
3. 불필요한 벤치 스크립트/벤치 바이너리 경로 제거
4. 스키마 관련 강제검증/관리 코드 잔존 여부 재검증
5. 포맷/테스트/스크립트 문법 검증

## Done Criteria
- 이름 변경 검토 결과와 근거가 문서화됨
- scripts가 `-p coclai` 하드코딩 없이 동작 가능
- `docs/` 경로만 남고 중복 문서 트리가 제거됨
- 스키마 관리 스크립트/강제검증 코드가 없음(프로토콜 `output_schema` 필드는 예외)
- 핵심 런타임 테스트 통과
