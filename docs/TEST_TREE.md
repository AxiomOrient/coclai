# TEST TREE

## 목적
- 테스트 의도를 `unit`, `contract`, `integration`으로 구분해 읽기/유지보수 비용을 낮춘다.
- 실서버 의존 테스트는 기본 파이프라인에서 제외하고 opt-in으로 분리한다.

## Layer 규칙
- `unit`: 순수 변환/모델/직렬화 규칙 검증. 외부 프로세스/네트워크 의존 없음.
- `contract`: JSON-RPC shape, 경계 검증, 소유권/격리 invariants 검증.
- `integration`: mock runtime 또는 실제 runtime wiring을 통한 end-to-end 흐름 검증.

## 모듈별 매핑
- `crates/codekko/src/adapters/web/tests`
  - `serialization` (unit)
  - `approval_boundaries` (contract)
  - `contract_and_spawn` (contract)
  - `approvals` (integration)
  - `routing_observability` (integration)
  - `session_flows` (integration)
- `crates/codekko/src/appserver/tests`
  - `contract` (unit)
  - `validated_calls` (contract; low-level typed parity entrypoints)
  - `server_requests` (integration)
- `crates/codekko/src/runtime/api/tests`
  - `params_and_types` (unit; wire shape, skills, command-exec, thread/turn override serialization)
  - `thread_api` (contract + integration; low-level thread/turn wrappers, override roundtrip, security boundaries)
  - `run_prompt` (integration)
- `crates/codekko/src/domain/artifact/tests.rs`
  - `unit_core`
  - `collect_output` (contract)
  - `runtime_tasks` (integration)
- `crates/codekko/src/ergonomic/tests`
  - `unit` (unit)
  - `real_server` (integration, opt-in only)
- `crates/codekko/src/plugin/tests`
  - `hook_report` (unit)
  - `contract_version` (contract)

## 중복 제거 원칙
- 동일 invariant를 여러 레이어에서 반복 검증하지 않는다.
- `unit`에서 충분히 보장되는 순수 변환 검증은 `integration`에서 재검증하지 않는다.
- `integration`은 교차 모듈 상호작용(상태/수명주기/경계 I/O)만 검증한다.
- 새 typed parity(`skills/list`, `command/exec*`, extended override)는 기본적으로 `unit + contract + mock integration`까지를 표준으로 하고, deterministic 실서버 트리거가 없으면 live gate로 올리지 않는다.

## 실행 가이드
- 기본 전체 세트:
  - `cargo test --workspace`
- opt-in 실서버 세트(9개 ignored 시나리오, release preflight 포함 가능):
  - `CODEKKO_REAL_SERVER_APPROVED=1 cargo test -p codekko ergonomic::tests::real_server:: -- --ignored --nocapture`
- 레이어별 예시:
  - `cargo test -p codekko runtime::api::tests::params_and_types:: -- --nocapture`
  - `cargo test -p codekko adapters::web::tests::contract_and_spawn:: -- --nocapture`
  - `cargo test -p codekko domain::artifact::tests::runtime_tasks:: -- --nocapture`
