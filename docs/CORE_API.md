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

## 기본 계약
1. `PromptRunParams::new` 기본 effort는 `medium`.
2. `run_prompt` 경로는 첨부 경로를 실행 전 검증.
3. 고수준 thread/turn API는 known-method validation 경로를 기본 사용.
4. `Session::close()` 이후 동일 세션 핸들의 `ask/interrupt`는 로컬에서 즉시 거절.
5. hook 실패는 fail-open(메인 흐름 유지 + 리포트 축적).

## 호환성/Deprecated
- `Client::setup*`, `continue_session*`, `interrupt_session_turn`, `close_session`는 deprecated alias.
- 신규 코드는 `start_session`/`Session::{ask,ask_with,close,interrupt_turn}` 또는 `Runtime` 저수준 API 사용 권장.

## 검증 명령
```bash
cargo fmt --all --check
cargo test -p coclai runtime::client::tests:: -- --nocapture
cargo test -p coclai runtime::core::tests:: -- --nocapture
cargo test -p coclai runtime::api::tests:: -- --nocapture
```
