# SCHEMA_AND_CONTRACT

`coclai`는 "app-server 래핑 + 라이프사이클 제어"를 핵심 계약으로 둔다.
이 문서는 코어 계약만 유지한다.

## 1) Schema Source of Truth

### 1.1 기준 경로

- 활성 스키마: `SCHEMAS/app-server/active/json-schema`
- 메타데이터: `SCHEMAS/app-server/active/metadata.json`
- 매니페스트: `SCHEMAS/app-server/active/manifest.sha256`
- 이벤트 골든: `SCHEMAS/golden/events/*.json`

### 1.2 스키마 갱신

```bash
./scripts/update_schema.sh
```

### 1.3 무결성 검사

```bash
./scripts/check_schema_manifest.sh
```

### 1.4 외부 드리프트 검사

```bash
./scripts/check_schema_drift.sh
```

- 기본 모드: `COCLAI_SCHEMA_DRIFT_MODE=soft` (warning only)
- 차단 모드: `COCLAI_SCHEMA_DRIFT_MODE=hard` (drift 시 non-zero exit)
- 릴리즈 preflight(`./scripts/release_preflight.sh`)는 `hard + source=codex`를 강제한다.

### 1.5 문서-코드 계약 동기화 검사

```bash
./scripts/check_doc_contract_sync.sh
```

- 기본 모드: `COCLAI_DOC_SYNC_MODE=hard` (링크 커버리지 100% 미달 시 non-zero exit)
- 완화 모드: `COCLAI_DOC_SYNC_MODE=soft` (warning only)
- strict mismatch 차단: `COCLAI_DOC_SYNC_FAIL_ON_MISMATCH=1` (mismatch verdict 존재 시 non-zero exit)
- 릴리즈 preflight(`./scripts/release_preflight.sh`)는 `COCLAI_DOC_SYNC_FAIL_ON_MISMATCH=1`을 강제한다.

### 1.6 스크립트 smoke harness

```bash
./scripts/smoke_script_harness.sh
```

- schema/doc 스크립트의 정상/주입 실패 분기를 smoke 검증한다.

런타임은 `spawn_local` 시점에 metadata/manifest를 fail-fast 검증한다.

## 2) Runtime/Lifecycle Contract

### 2.1 라이프사이클 순서

1. `Client::connect_*`
2. `run(...)` 또는 `setup(...)`
3. `Session::ask(...)` 반복
4. `Session::close(...)`
5. `Client::shutdown(...)`

### 2.2 상태 전이 규칙

1. `Session::close()` 이후 동일 핸들의 `ask/interrupt`는 즉시 에러여야 한다.
2. `run_prompt` 계열은 첨부 경로를 실행 전에 검증해야 한다.
3. 기본 effort는 `medium`이어야 한다.

### 2.3 Hook 계약(구현 상태)

현재(2026-02-26) 기준으로 아래 Hook 계약은 구현 완료 상태다.

1. Hook 실행 순서:
  - `pre_*` -> core call -> `post_*`
2. pre Hook은 입력 변형을 허용한다.
3. Hook 실패 정책은 fail-open이다.
  - hook 에러는 `HookReport`로 전파
  - 메인 AI 작업은 계속 진행
4. Cross-crate C-lite 계약:
  - 공통 core contract는 lifecycle 축만 가진다.
  - artifact/web는 adapter contract로 연결한다.
  - 도메인 전용 필드를 core trait에 강제하지 않는다.
  - `PluginContractVersion` major 불일치 시 명시적 호환성 오류를 반환한다.
    - artifact: `DomainError::IncompatibleContract`
    - web: `WebError::IncompatibleContract`
5. Hook 미설정 경로는 기존 동작과 동일해야 한다.

## 3) Contract Tests (Core)

### 3.1 결정적 계약 테스트 (기본)

```bash
cargo test -p coclai_runtime --test contract_deterministic -- --nocapture
```

### 3.2 실제 CLI smoke (옵션)

```bash
APP_SERVER_CONTRACT=1 \
cargo test -p coclai_runtime --test contract_real_cli -- --nocapture
```

### 3.3 nightly/opt-in 통합 게이트 (옵션)

```bash
./scripts/run_nightly_opt_in_gate.sh
```

- 실행 결과 로그:
  - `target/qa/nightly_opt_in/<timestamp>/script_smoke.log`
  - `target/qa/nightly_opt_in/<timestamp>/real_cli_contract.log`

## 4) Performance Gate (Local)

### 4.1 baseline 생성

```bash
CODEX_PERF_WRITE_BASELINE=1 ./scripts/run_micro_bench.sh
```

### 4.2 회귀 체크

```bash
./scripts/run_micro_bench.sh
```

- 기본 임계치: p95 15%
- Hook 선형성 임계치: `h3 <= h1*3*(1+slack)`, `h5 <= h1*5*(1+slack)`
  - 기본 slack: 50% (`CODEX_PERF_MAX_HOOK_LINEARITY=0.50`)
- 기본 재시도: `CODEX_PERF_RETRIES=2`
- baseline: `SCHEMAS/golden/perf/micro_bench_baseline.json`
- 결과 리포트:
  - `hookPreH0/hookPreH1/hookPreH3/hookPreH5`
  - `adapterDirectSync/adapterDynSync/adapterOverhead`
