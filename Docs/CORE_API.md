# CORE_API

`coclai`의 공개면은 Agent-first 기준으로 재정의되었습니다.

## 0) 브레이킹 기준

- 버전: `0.2.0`
- 정책: 호환성 유지/마이그레이션 경로를 제공하지 않습니다.
- 외부 1급 통합 경로: `coclai-agent` ingress(`stdio`, `http(localhost)`, `ws(localhost)`).

## 1) 1급 공개면 (권장)

### 1.1 Agent 타입

```rust
pub use agent::{
    AgentDispatchError,
    AgentHealth,
    AgentSecurityPolicy,
    CapabilityInvocation,
    CapabilityResponse,
    CoclaiAgent,
};
```

의미:
- `CapabilityInvocation`: ingress 공통 요청 envelope
- `CapabilityResponse`: ingress 공통 응답 envelope
- `CoclaiAgent::dispatch`: 모든 ingress가 수렴하는 단일 디스패치 진입점

### 1.2 Capability registry/parity

```rust
pub use capability::{
    capability_by_id,
    capability_parity_gaps,
    capability_registry,
    missing_capabilities_for_ingress,
    render_capability_parity_report,
    CapabilityDescriptor,
    CapabilityExposure,
    CapabilityIngress,
    CapabilityIngressSupport,
};
```

운영 기준:
- registry 전 항목이 `stdio/http/ws`에서 동일 의미(success/failure semantics)로 동작해야 합니다.

## 2) ingress 계약

### 2.1 stdio

- `coclai-agent invoke <capability_id> --ingress stdio --payload <json>`
- capability 실행 결과를 JSON으로 반환합니다.

### 2.2 HTTP(localhost)

- `GET /health` -> `system/health`
- `GET /capabilities` -> `system/capability_registry`
- `POST /invoke` -> 임의 capability envelope

요청 보안:
- loopback caller만 허용
- 토큰(`COCLAI_AGENT_TOKEN`) 필수일 때 헤더로 전달
  - `x-coclai-token: <token>`
  - 또는 `Authorization: Bearer <token>`

### 2.3 WebSocket(localhost)

- `GET /ws`
- 메시지 단위 JSON invocation envelope
- HTTP와 동일한 인증/권한 의미를 유지합니다.

## 3) 비공개 전환된 레거시 보조면

0.2.0 기준 아래 경로는 root 공개면에서 제거되었습니다.

- `quick_run`, `quick_run_with_profile`
- `Workflow`, `WorkflowConfig`
- `AppServer`, `rpc_methods`
- `Client`/`Runtime` 계열 broad facade

원칙:
- 신규 통합은 capability ingress(또는 `CoclaiAgent::dispatch`)로만 구성합니다.
- 위 경로는 내부 구현 세부사항으로 취급하며 공개 호환성 대상이 아닙니다.

## 4) 헥사고날 경계

단일 crate 내부 구조:

```text
crates/coclai/src/
  domain/
  application/
  ports/
  adapters/
  bootstrap/
  bin/coclai_agent.rs
```

강제 규칙:
- `domain`에서 `axum`, `tokio`, `std::process::Command` 금지
- `application`에서 `adapters::` 직접 참조 금지
- 정적 검사: `scripts/check_hexagonal_boundaries.sh`

## 5) 릴리즈 최소 게이트

```bash
bash scripts/check_hexagonal_boundaries.sh
cargo check -p coclai
cargo clippy -p coclai --all-targets -- -D warnings
cargo test -p coclai --lib --tests
bash scripts/release_agent_go_no_go.sh
COCLAI_DOC_SYNC_MODE=hard COCLAI_DOC_SYNC_FAIL_ON_MISMATCH=1 bash scripts/check_doc_contract_sync.sh
```

## 6) 참고 문서

- `README.md`
- `Docs/IMPLEMENTATION-PLAN.md`
- `Docs/TASKS.md`
