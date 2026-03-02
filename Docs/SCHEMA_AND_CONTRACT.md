# SCHEMA_AND_CONTRACT

`coclai`는 단일 monolith 런타임에서 app-server 계약을 유지합니다.

## 1) Schema SoT

- 활성 스키마: `SCHEMAS/app-server/active/json-schema`
- 메타데이터: `SCHEMAS/app-server/active/metadata.json`
- 매니페스트: `SCHEMAS/app-server/active/manifest.sha256`
- 이벤트 골든: `SCHEMAS/golden/events/*.json`

갱신/검증:

```bash
./scripts/update_schema.sh
./scripts/check_schema_manifest.sh
./scripts/check_schema_drift.sh
```

문서-코드 동기:

```bash
./scripts/check_doc_contract_sync.sh
```

## 2) Runtime/Lifecycle Contract

1. `Client::connect_*`
2. `run(...)` 또는 `setup(...)`
3. `Session::ask(...)` 반복
4. `Session::close(...)`
5. `Client::shutdown(...)`

핵심 규칙:
- `Session::close()` 이후 동일 핸들의 `ask/interrupt`는 즉시 실패해야 한다.
- `run_prompt` 계열은 첨부 경로를 실행 전에 검증해야 한다.
- 기본 effort는 `medium`이다.

## 3) Agent Capability Contract

- 외부 통합은 `coclai-agent` ingress(`stdio/http/ws`)를 사용한다.
- 공통 envelope는 `CapabilityInvocation` / `CapabilityResponse`를 사용한다.
- registry capability 전 항목이 ingress 3종에서 동일 의미를 유지해야 한다.

## 4) 테스트 게이트

기본 회귀:

```bash
cargo test -p coclai --lib --tests -- \
  --skip ergonomic::tests::real_server::quick_run_executes_prompt_against_real_codex_server \
  --skip ergonomic::tests::real_server::workflow_run_executes_prompt_against_real_codex_server
```

실서버 lane(옵션):

```bash
cargo test -p coclai ergonomic::tests::real_server::quick_run_executes_prompt_against_real_codex_server -- --nocapture
cargo test -p coclai ergonomic::tests::real_server::workflow_run_executes_prompt_against_real_codex_server -- --nocapture
```

야간 옵트인:

```bash
./scripts/run_nightly_opt_in_gate.sh
```

## 5) 릴리즈 최소 게이트

```bash
bash scripts/check_hexagonal_boundaries.sh
cargo check -p coclai
cargo clippy -p coclai --all-targets -- -D warnings
bash scripts/release_agent_go_no_go.sh
COCLAI_DOC_SYNC_MODE=hard COCLAI_DOC_SYNC_FAIL_ON_MISMATCH=1 bash scripts/check_doc_contract_sync.sh
```
