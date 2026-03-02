# ARCHITECTURE

`coclai`는 **단일 런타임 crate(`crates/coclai`)** 내부에서 헥사고날 경계로 분리된 agent-first monolith입니다.

## 1) 계층 경계

### 1.1 domain
- 책임: capability/session/thread/turn/approval/artifact/runtime_state 순수 모델과 규칙
- 비책임: 네트워크, 프로세스 spawn, tokio/axum 의존

### 1.2 application
- 책임: use-case 조합(`capability_dispatch`, `quick_run`, `workflow`, `appserver`, `thread_turn`)
- 비책임: adapter concrete 직접 참조

### 1.3 ports
- 책임: inbound/outbound trait 계약 정의
- 비책임: 구현체/런타임 I/O

### 1.4 adapters
- inbound: `cli/http/ws/stdio` 입력 파싱, 인증, 에러 변환
- outbound: `codex_stdio/memory_store` I/O 구현

### 1.5 bootstrap/bin
- `bootstrap`: container 조립점
- `bin/coclai_agent.rs`: orchestration only

## 2) Agent-first 표준 경로

- 외부 통합은 `coclai-agent`를 통해 수행
- ingress: `stdio`, `HTTP(localhost)`, `WS(localhost)`
- 공통 계약: `CapabilityInvocation` / `CapabilityResponse`
- 보안 정책: loopback + token
- root 공개 Rust API는 `build_agent`, `CoclaiAgent`, capability registry/parity 집합으로 최소화

## 3) 데이터 우선 모델

핵심 상태/메시지는 명시적 타입으로 고정한다.

- 이벤트: `Envelope`
- 런타임 상태: `RuntimeState`, `ThreadState`, `TurnState`, `ItemState`
- 승인 요청: `ServerRequest`
- 계측: `RuntimeMetricsSnapshot`
- agent capability envelope: `CapabilityInvocation`, `CapabilityResponse`

## 4) 강제 규칙

1. `domain`에서 `axum/tokio/std::process::Command` import 금지
2. `application`에서 `adapters::` 직접 참조 금지
3. ingress adapter는 파싱/검증/에러 변환만 수행
4. 비즈니스 분기 로직은 application으로 수렴
5. 외부 I/O는 outbound adapter/infrastructure로 제한

정적 검사:
- `scripts/check_hexagonal_boundaries.sh`

## 5) 릴리즈 게이트

- release gate: `scripts/release_agent_go_no_go.sh`
