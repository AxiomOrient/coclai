# Big-Bang Cutover Operations

작성일: 2026-02-28
대상: Runtime/Web/Artifact/Facade 재구조화 컷오버

## 1) Cutover Preconditions

아래 항목이 모두 `PASS`일 때만 컷오버를 진행한다.

1. `bash scripts/release_preflight.sh`
2. `bash scripts/run_micro_bench.sh`
3. `bash scripts/run_nightly_opt_in_gate.sh`
4. `COCLAI_DOC_SYNC_MODE=hard COCLAI_DOC_SYNC_FAIL_ON_MISMATCH=1 bash scripts/check_doc_contract_sync.sh`
5. `rg -n "todo!\\(|unimplemented!\\(" crates` 결과 0
6. `rg -n "TODO" crates --glob '!**/tests/fixtures/**'` 결과 0

## 2) Rehearsal Record (2026-02-28)

- `release_preflight.sh`: PASS
- `run_micro_bench.sh`: PASS
  - regression check: threshold 이내 통과
- `run_nightly_opt_in_gate.sh`: PASS
  - log root: `target/qa/nightly_opt_in/20260228T084839Z`
  - smoke log: `target/qa/nightly_opt_in/20260228T084839Z/script_smoke.log`
  - real-cli log: `target/qa/nightly_opt_in/20260228T084839Z/real_cli_contract.log`

## 3) Cutover Execution Checklist

1. 배포 전 태그 생성 (`pre-bigbang-cutover-YYYYMMDD`).
2. preflight/bench/nightly 결과 재확인.
3. 배포 윈도우 시작 공지.
4. 단일 릴리즈 배포.
5. 30분 집중 관측 (error rate, p95 latency, runtime restart count).
6. 24시간 안정화 관측 전환.

## 4) Rollback Policy

### 4.1 Rollback Triggers

아래 중 하나라도 충족하면 즉시 롤백한다.

1. 5분 이동창 기준 API error rate > 2%
2. p95 latency > baseline 대비 35% 초과가 10분 이상 지속
3. runtime restart loop 감지(연속 3회 이상)
4. tenant/session isolation 위반 또는 approval ownership 위반 1건 이상
5. doc-contract sync 혹은 schema drift hard gate 재실행 실패

### 4.2 Rollback Actions

1. 직전 안정 태그로 즉시 되돌림.
2. 롤백 후 `release_preflight.sh`를 다시 실행해 정상 상태 확인.
3. 장애 원인 이벤트를 분류(runtime/web/artifact/facade).
4. 원인 분석 완료 전 재배포 금지.

## 5) SLO and Monitoring Matrix

| Metric | Target/SLO | Alert Condition | Owner |
|---|---|---|---|
| API error rate | <= 1% (5m) | > 2% (5m) | Runtime on-call |
| p95 latency | baseline + <= 20% | baseline + > 35% for 10m | Runtime on-call |
| Runtime restart count | 0 정상 | 3회 연속 restart | Runtime on-call |
| Approval timeout ratio | <= 0.5% | > 1% (15m) | Web on-call |
| Artifact conflict ratio | <= 2% | > 5% (15m) | Artifact on-call |
| Doc contract drift | mismatch 0 | mismatch > 0 | Release manager |

## 6) Post-Cutover Stabilization

1. T+30m: 집중 모니터링 종료 판단
2. T+2h: 오류/성능 회고 1차
3. T+24h: 안정화 종료 및 릴리즈 노트 확정
4. T+48h: 리팩토링 후속 최적화 backlog 분리
