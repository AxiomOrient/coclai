# CORE_API

## 공개 경로

### 고수준 (`coclai`)
- `quick_run`, `quick_run_with_profile`
- `Workflow`, `WorkflowConfig`
- `AppServer`

### 런타임 (`coclai::runtime`)
- `Client`, `Session`, `Runtime`
- `ClientConfig`, `RunProfile`, `SessionConfig`
- `ApprovalPolicy`, `SandboxPolicy`, `ReasoningEffort`
- `RpcValidationMode`, `RpcError`, `RuntimeError`

## 내부 단일 경로 (`2026-03-07`)
1. turn lifecycle은 `runtime/turn_lifecycle.rs` 공용 엔진으로 수렴한다.
2. `runtime/api` 보조 계층은 `thread_api.rs` + `wire.rs` 단일 경로를 사용한다(`ops.rs` 제거).
3. sandbox 정책 직렬화/검증은 `sandboxPolicy` 단일 경로만 사용한다.

## 기본 계약
1. `PromptRunParams::new` 기본 effort는 `medium`.
2. `run_prompt` 경로는 첨부 경로를 실행 전 검증.
3. `run_prompt` timeout은 절대 deadline을 상한으로 강제한다(lag fallback 포함).
4. thread/turn id 파싱은 canonical field만 허용한다(loose fallback 금지).
5. 고수준 thread/turn API는 known-method validation 경로를 기본 사용.
6. `Session::close()`는 single-flight를 보장하며 close 후 동일 핸들의 `ask/interrupt`를 로컬 거절한다.
7. hook 실패는 fail-open(메인 흐름 유지 + 리포트 축적) 정책을 유지한다.

## 호환성 정책
- `deprecated` alias/호환 레이어를 유지하지 않는다(big-bang 정책).
- 세션 제어는 `start_session` + `Session::{ask,ask_with,close,interrupt_turn}` 단일 경로를 사용한다.
- 실서버 E2E는 기본 파이프라인에서 제외하고 opt-in으로만 실행한다.

## Opt-In 실서버 최종 검증 절차
기본 원칙:
1. 자동 파이프라인에서는 실서버 테스트를 절대 실행하지 않는다.
2. 운영자(사람) 승인 이후에만 `COCLAI_REAL_SERVER_APPROVED=1`로 수동 실행한다.
3. 승인 없이 실서버 게이트를 활성화하면 실패해야 한다.

사전 조건:
1. 로컬에서 기본 게이트가 먼저 통과되어야 한다.
2. `codex app-server`에 연결 가능한 환경이어야 한다.
3. 현재 셸에서 `COCLAI_REAL_SERVER_APPROVED=1`을 명시적으로 설정한다.

직접 실행(실서버 시나리오 7종: oneshot/workflow/attachment/session/resume/appserver/approval):
```bash
COCLAI_REAL_SERVER_APPROVED=1 \
cargo test -p coclai ergonomic::tests::real_server:: -- --ignored --nocapture
```

릴리즈 프리플라이트 경로:
```bash
COCLAI_RELEASE_INCLUDE_REAL_SERVER=1 \
COCLAI_REAL_SERVER_APPROVED=1 \
./scripts/release_preflight.sh
```

선택 옵션(재시도/백오프):
1. `COCLAI_RELEASE_REAL_SERVER_RETRIES` (기본 `3`)
2. `COCLAI_RELEASE_REAL_SERVER_BACKOFF_SEC` (기본 `3`)

실행 후 정리:
```bash
unset COCLAI_REAL_SERVER_APPROVED
unset COCLAI_RELEASE_INCLUDE_REAL_SERVER
unset COCLAI_RELEASE_REAL_SERVER_RETRIES
unset COCLAI_RELEASE_REAL_SERVER_BACKOFF_SEC
```

현재 live capability boundary:
1. 검증된 opt-in 실서버 흐름은 `quick_run`, `workflow.run`, attachment 포함 `quick_run_with_profile`, `workflow.setup_session -> ask`, `client.resume_session -> ask`, low-level `AppServer` thread roundtrip, `AppServer` approval roundtrip의 7개다.
2. approval live 시나리오는 trusted workspace에서는 쓰기 승인이 자동 승인될 수 있으므로, `/tmp` 아래 untrusted scratch dir에서만 재현 가능하도록 고정한다.
3. `requestUserInput`은 현재 실서버 모드에서 `item/tool/requestUserInput`을 보내지 않고 거절 응답으로 끝나므로 live gate에 넣지 않는다.
4. dynamic tool-call은 runtime 라우팅/타임아웃/검증 경로는 존재하지만, 현재 래퍼 표면에는 실서버에서 결정적으로 유도할 client tool registration 경로가 없어 mock/unit 검증만 유지한다.

## 검증 명령
```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
./scripts/check_blocker_regressions.sh
./scripts/check_security_gate.sh
./scripts/check_product_hygiene.sh
COCLAI_REAL_SERVER_APPROVED=1 COCLAI_RELEASE_INCLUDE_REAL_SERVER=1 ./scripts/release_preflight.sh
```
