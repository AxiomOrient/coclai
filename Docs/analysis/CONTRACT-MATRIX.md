# Contract Matrix (Consolidated)

본 문서는 Phase 1 분석 결과 중 운영에 필요한 핵심 산출물만 보존한 축약본이다.

- Source snapshot: `T-022` declaration vs implementation matrix
- Primary consumer: `scripts/check_doc_contract_sync.sh`

## Declaration vs Implementation Matrix (T-022 Deliverable)

### Verdict Legend

- `match`: 문서 선언과 구현/테스트 근거가 직접 대응됨
- `mismatch`: 문서 선언과 구현이 충돌하거나 구현 근거가 반대 방향임
- `uncertain`: 선언은 있으나 구현에서 강제/검증 근거가 충분히 닫히지 않음

### README (`CON-*`) Matrix

| Contract ID | Verdict | Implementation Evidence | Note |
|---|---|---|---|
| CON-001 | match | `crates/coclai/src/lib.rs:1-3`, `crates/coclai/src/lib.rs:7-23` | facade crate가 runtime wrapper를 재노출 |
| CON-002 | match | `crates/coclai/src/ergonomic.rs:276-299`, `crates/coclai/src/ergonomic.rs:194-256` | one-shot + explicit lifecycle API 존재 |
| CON-003 | match | `crates/coclai_runtime/src/hooks.rs:121-157`, `crates/coclai_runtime/src/api.rs:1451-1506` | pre/post hook 체인 실행 구현 |
| CON-004 | match | `crates/coclai_runtime/src/rpc_contract.rs:21-108`, `scripts/release_preflight.sh:7-21` | RPC 검증 + 릴리즈 게이트 구현 |
| CON-005 | match | `crates/coclai/src/ergonomic.rs:15-33`, `crates/coclai/src/ergonomic.rs:276-299` | beginner/advanced 경로 동시 제공 |
| CON-006 | match | `crates/coclai/src/appserver.rs:23-203` | AppServer direct facade 구현 |
| CON-007 | match | `crates/coclai/src/appserver.rs:10-21` | thread/turn 메서드 상수 제공 |
| CON-008 | match | `crates/coclai_runtime/src/api.rs:1451-1461`, `crates/coclai_runtime/src/api.rs:1492-1506` | pre -> core -> post 순서 유지 |
| CON-009 | match | `crates/coclai_runtime/src/api.rs:990-1014`, `crates/coclai_plugin_core/src/lib.rs:69-74` | prompt/model/attachments/metadata_delta 변형 허용 |
| CON-010 | match | `crates/coclai_runtime/src/hooks.rs:132-139`, `crates/coclai_runtime/src/hooks.rs:153-156` | hook 오류는 report에 기록되고 실행 계속 |
| CON-011 | match | `crates/coclai_runtime/src/client.rs:148-166`, `crates/coclai_runtime/src/client.rs:1002-1010` | with_schema_dir -> env -> cwd -> package 순서 |
| CON-012 | match | `crates/coclai_runtime/src/client.rs:58-63`, `crates/coclai_runtime/src/client.rs:957-960` | 기본 guard가 userAgent를 요구 |
| CON-013 | match | `crates/coclai_runtime/src/client.rs:23-27`, `crates/coclai_runtime/src/client.rs:968-974` | 최소 버전 0.104.0 검사 |
| CON-014 | match | `crates/coclai_runtime/src/client.rs:490-543`, `crates/coclai_runtime/src/client.rs:897-910` | close 후 ask/interrupt 즉시 거절 |
| CON-015 | match | `Cargo.toml:12`, `crates/coclai/Cargo.toml:4` | workspace/crates edition 2021 |
| CON-016 | uncertain | `crates/coclai_runtime/src/client.rs:78`, `crates/coclai_runtime/src/client.rs:580-583` | `codex` 실행 전제는 코드에 있으나 로그인 상태는 외부 환경 의존 |
| CON-017 | match | `crates/coclai_runtime/src/runtime_schema.rs:12-15`, `crates/coclai_runtime/src/runtime_schema.rs:16-44` | metadata/manifest/json-schema fail-fast 검증 |
| CON-018 | match | `scripts/release_preflight.sh:7-21`, `scripts/check_product_hygiene.sh:7-27` | fmt/clippy/hygiene/tests/schema manifest 게이트 |
| CON-019 | match | `crates/coclai_runtime/src/client.rs:919-922`, `crates/coclai_runtime/src/client.rs:1017-1027` | 스키마 경로 오류 타입 명시 |
| CON-020 | match | `crates/coclai_runtime/src/client.rs:924-935`, `crates/coclai_runtime/src/client.rs:114-121` | 버전/UA 오류 및 guard 비활성화 경로 존재 |
| CON-021 | match | `crates/coclai_runtime/src/client.rs:549-562`, `crates/coclai_runtime/src/client.rs:906-910` | 종료 후 호출 거절이 계약화 |

### ARCHITECTURE (`ARC-*`) Matrix

| Contract ID | Verdict | Implementation Evidence | Note |
|---|---|---|---|
| ARC-001 | uncertain | `crates/` 디렉터리(5 crates), `crates/coclai_plugin_core/src/lib.rs:10-25` | 문서 4계층 외 shared plugin_core 존재로 계층 모델 해석 여지 |
| ARC-002 | match | `crates/coclai/src/lib.rs:4-23` | facade는 re-export 중심 |
| ARC-003 | uncertain | `crates/coclai/src/lib.rs:4-23`, `crates/coclai/src/appserver.rs:23-203` | 비책임(negative claim) 완전 증명은 어려우나 transport 구현은 runtime에 위치 |
| ARC-004 | match | `crates/coclai_runtime/src/runtime.rs:198-223`, `crates/coclai_runtime/src/runtime/dispatch.rs:24-143` | transport/dispatch/approval/state/metrics 책임 구현 |
| ARC-005 | match | `crates/coclai_artifact/src/lib.rs:63-137`, `crates/coclai_artifact/src/patch.rs:23-77` | doc patch 모델/검증/store 연동 |
| ARC-006 | match | `crates/coclai_web/src/lib.rs:215-352`, `crates/coclai_web/src/state.rs:95-136` | tenant/session 분리 + SSE + approval bridge |
| ARC-007 | match | `crates/coclai_runtime/src/rpc.rs:24-47`, `crates/coclai_runtime/src/state.rs:108-131`, `crates/coclai_artifact/src/patch.rs:23-77`, `crates/coclai_runtime/src/approvals.rs:73-83` | 순수 변환 함수들이 I/O 없이 동작 |
| ARC-008 | match | `crates/coclai_runtime/src/runtime/state_projection.rs:37-40`, `crates/coclai_runtime/src/runtime/dispatch.rs:35-141` | side-effect 경계에서 pure transform 결과 적용 |
| ARC-009 | match | `crates/coclai_runtime/src/runtime/dispatch.rs:34-143` | reader->classify->resolve/queue->envelope->reduce->sink->broadcast 구현 |
| ARC-010 | uncertain | `crates/coclai_runtime/src/runtime.rs:159-166`, `crates/coclai_runtime/src/state.rs:124-126`, `crates/coclai_runtime/src/runtime/dispatch.rs:181-207` | 평균 O(1)은 코드 구조상 타당하나 상한/실측 보장은 별도 |
| ARC-011 | uncertain | `crates/coclai_runtime/src/state.rs:124-126`, `crates/coclai_runtime/src/runtime/dispatch.rs:126-141` | deep copy 금지 정책은 문서 규칙이며 정적 강제는 없음 |
| ARC-012 | match | `crates/coclai_runtime/src/runtime/dispatch.rs:186-205`, `crates/coclai_runtime/src/metrics.rs:128-133` | sink 포화 시 drop + 계측, core path 비차단 |

### CORE_API (`API-*`) Matrix

| Contract ID | Verdict | Implementation Evidence | Note |
|---|---|---|---|
| API-001 | match | `crates/coclai/src/lib.rs:8-21`, `crates/coclai_runtime/src/api.rs:1451-1506` | lifecycle + hook 체인 API 노출 |
| API-002 | match | `crates/coclai_runtime/src/api.rs:1067-1087`, `crates/coclai_runtime/src/hooks.rs:132-139` | pre mutation 허용 + fail-open |
| API-003 | uncertain | `crates/coclai/src/lib.rs:1-3`, `crates/coclai/src/ergonomic.rs:185-199` | 권장 경로는 문서 정책(코드 강제 아님) |
| API-004 | match | `crates/coclai/src/ergonomic.rs:276-299` | quick_run one-shot path 구현 |
| API-005 | match | `crates/coclai/src/ergonomic.rs:26-33`, `crates/coclai/src/ergonomic.rs:319-329` | 상대경로 절대화 + fs 존재 검사 없음 |
| API-006 | match | `crates/coclai/src/appserver.rs:10-21`, `crates/coclai/src/appserver.rs:34-203` | AppServer + rpc_methods 제공 |
| API-007 | match | `crates/coclai_runtime/src/lib.rs:1-50` | core type root re-export + helper module path 접근 가능 |
| API-008 | uncertain | `Docs/CORE_API.md:128` | SoT 위임은 문서 간 계약으로 코드 단독 검증 불가 |
| API-009 | match | `crates/coclai_runtime/src/api.rs:136`, `crates/coclai_runtime/src/api.rs:565-575` | default effort = medium |
| API-010 | match | `crates/coclai_runtime/src/api.rs:1432`, `crates/coclai_runtime/src/api/wire.rs:10-29` | run_prompt 경로 첨부 경로 사전 검증 |
| API-011 | match | `crates/coclai_runtime/src/client.rs:549-562`, `crates/coclai_runtime/src/client.rs:897-910` | close 후 거절 + close 결과 캐시 |
| API-012 | match | `crates/coclai_runtime/src/client.rs:593-603` | compatibility 실패 시 shutdown 실패도 에러 전파 |
| API-013 | match | `crates/coclai_runtime/src/runtime.rs:587-603` | shutdown join 실패 -> RuntimeError::Internal |
| API-014 | match | `crates/coclai_runtime/src/client.rs:124-141`, `crates/coclai_runtime/src/client.rs:295-310`, `crates/coclai_runtime/src/client.rs:450-465` | 3개 config 모델 모두 hook builder 제공 |
| API-015 | match | `crates/coclai_artifact/src/lib.rs:443-451`, `crates/coclai_artifact/src/lib.rs:456-463` | artifact major mismatch -> DomainError::IncompatibleContract |
| API-016 | match | `crates/coclai_web/src/adapter.rs:68-75`, `crates/coclai_web/src/adapter.rs:159-163`, `crates/coclai_web/src/lib.rs:371-383` | Runtime당 1 web binding + contract mismatch 오류 |

### SCHEMA_AND_CONTRACT (`SCC-*`) Matrix

| Contract ID | Verdict | Implementation Evidence | Note |
|---|---|---|---|
| SCC-001 | match | `crates/coclai/src/lib.rs:1-3`, `crates/coclai_runtime/src/client.rs:565-793` | app-server wrapping + lifecycle API 구현 |
| SCC-002 | match | `SCHEMAS/app-server/active/*`, `SCHEMAS/golden/events/*` | 문서 경로와 실제 트리 일치 |
| SCC-003 | match | `scripts/update_schema.sh:11-66` | 스키마 갱신 스크립트 존재 |
| SCC-004 | match | `scripts/check_schema_manifest.sh:4-36` | 무결성 검사 스크립트 존재 |
| SCC-005 | match | `crates/coclai_runtime/src/runtime.rs:215-223`, `crates/coclai_runtime/src/runtime_schema.rs:11-44` | spawn_local 시 schema guard fail-fast |
| SCC-006 | match | `crates/coclai_runtime/src/client.rs:565-793`, `crates/coclai_runtime/src/client.rs:470-562` | connect/run/setup/ask/close/shutdown 경로 API 제공 |
| SCC-007 | match | `crates/coclai_runtime/src/client.rs:490-543`, `crates/coclai_runtime/src/client.rs:897-910` | close 이후 ask/interrupt 즉시 에러 |
| SCC-008 | match | `crates/coclai_runtime/src/api.rs:1432`, `crates/coclai_runtime/src/api/wire.rs:10-29` | run_prompt 계열 사전 첨부 검증 |
| SCC-009 | match | `crates/coclai_runtime/src/api.rs:136`, `crates/coclai_runtime/src/api.rs:565-575` | 기본 effort medium |
| SCC-010 | match | `crates/coclai_runtime/src/hooks.rs:121-157`, `crates/coclai_runtime/src/api.rs:1451-1506` | Hook 계약 경로 구현 완료 |
| SCC-011 | match | `crates/coclai_runtime/src/api.rs:1451-1461`, `crates/coclai_runtime/src/api.rs:1492-1506` | pre -> core -> post 순서 |
| SCC-012 | match | `crates/coclai_plugin_core/src/lib.rs:69-74`, `crates/coclai_runtime/src/api.rs:990-1014` | pre hook mutation 허용 |
| SCC-013 | match | `crates/coclai_runtime/src/hooks.rs:132-139`, `crates/coclai_runtime/src/hooks.rs:153-156` | fail-open + HookReport |
| SCC-014 | match | `crates/coclai_plugin_core/src/lib.rs:27-132`, `crates/coclai_artifact/src/adapter.rs:19-35`, `crates/coclai_web/src/adapter.rs:23-52` | core 공통 계약 + adapter 연결 |
| SCC-015 | match | `crates/coclai_artifact/src/lib.rs:443-451`, `crates/coclai_web/src/lib.rs:371-383` | major mismatch 명시 오류 |
| SCC-016 | uncertain | `crates/coclai_runtime/src/api.rs:1431-1445`, `crates/coclai_runtime/src/api.rs:1528-1544` | no-hook path 존재하나 "기존 동작과 동일"의 역사적 동치 검증은 별도 필요 |
| SCC-017 | match | `crates/coclai_runtime/tests/contract_deterministic.rs:199-237` | deterministic contract suite 존재 |
| SCC-018 | match | `crates/coclai_runtime/tests/contract_real_cli.rs:50-56`, `crates/coclai_runtime/tests/contract_real_cli.rs:92-94` | APP_SERVER_CONTRACT=1 opt-in smoke |
| SCC-019 | match | `scripts/run_micro_bench.sh:9-13`, `scripts/run_micro_bench.sh:33-67`, `crates/coclai_runtime/src/bin/perf_micro_bench.rs:49-70` | p95 15%, hook 선형성, retries=2 구성 |

### SECURITY (`SEC-*`) Matrix

| Contract ID | Verdict | Implementation Evidence | Note |
|---|---|---|---|
| SEC-001 | uncertain | `Docs/SECURITY.md:3`, `crates/coclai_runtime/src/client.rs:187-197` | 계층 정책 선언은 있으나 전체 시스템 강제 수준은 정책/운영 의존 |
| SEC-002 | match | `crates/coclai_runtime/src/client.rs:191-194`, `crates/coclai_runtime/src/approvals.rs:44-50` | approval/sandbox/auto_decline_unknown 기본값 일치 |
| SEC-003 | uncertain | `crates/coclai_runtime/src/client.rs:191-194` | 목적(고위험 차단)은 설계 의도이며 효과 계측 근거는 별도 필요 |
| SEC-004 | match | `crates/coclai_runtime/src/api/wire.rs:34-63`, `crates/coclai_runtime/src/api/wire.rs:66-90`, `crates/coclai_runtime/src/api/tests.rs:1385-1438` | privileged sandbox는 explicit opt-in + non-never approval + explicit scope를 강제 |
| SEC-005 | match | `crates/coclai_runtime/src/client.rs:191-194`, `crates/coclai_runtime/src/runtime/dispatch.rs:243-250` | 기본 danger-full-access 아님 + timeout 기본 decline |
| SEC-006 | match | `crates/coclai_web/src/state.rs:41-43`, `crates/coclai_web/src/state.rs:106-108`, `crates/coclai_web/src/tests.rs:642-675` | tenant 교차 접근 차단 구현/테스트 |
| SEC-007 | match | `crates/coclai_web/src/wire.rs:39-59`, `crates/coclai_web/src/tests.rs:414-445` | SSE 직렬화에서 `rpcId`/response `json.id`를 redaction해 외부 노출을 차단 |
| SEC-008 | match | `crates/coclai_web/src/lib.rs:293-320`, `crates/coclai_web/src/lib.rs:329-340` | session 소유권 + approval owner 검증 |
| SEC-009 | uncertain | `crates/coclai_runtime/src/runtime/dispatch.rs:192-204`, `crates/coclai_runtime/src/runtime/dispatch.rs:224-229` | 민감값 최소화 정책은 일부 로그에 반영되나 전역 redaction 정책은 미확정 |
| SEC-010 | uncertain | `crates/coclai_artifact/src/store.rs`, `crates/coclai_web/src/state.rs:127-133` | 보존기간/redaction/접근제어 정책을 전역 규칙으로 강제하는 증거 부족 |

## T-022 Summary

- Total declarations evaluated: `78`
- `match`: `66`
- `mismatch`: `0`
- `uncertain`: `12`

## Mismatch Backlog Seeds

- 현재 열린 mismatch 항목 없음 (`SEC-004`, `SEC-007` closure 반영).
