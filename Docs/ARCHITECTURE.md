# ARCHITECTURE

`coclai`는 1개 파사드 + 3개 실행 계층으로 구성됩니다.

## 1) 계층 경계 (MECE)

### 1.1 Public Facade (`coclai`)
- 책임:
  - 기본 사용자 진입점 제공
  - 고수준 API만 재노출
  - 저수준 제어로 가는 명시적 통로 제공(`coclai::runtime`)
- 비책임:
  - transport/rpc 구현
  - 도메인 patch 로직
  - 웹 세션 라우팅

### 1.2 Core Runtime (`coclai_runtime`)
- 책임:
  - stdio(JSONL) transport
  - JSON-RPC dispatch/pending
  - approval routing
  - state projection (`RuntimeState`)
  - metrics snapshot
- 비책임:
  - 영속 저장소 정책
  - 웹 인증/인가
  - 도메인 프롬프트 설계

### 1.3 Domain Layer (`coclai_artifact`)
- 책임:
  - 문서/규칙 작업 spec 모델링
  - DocPatch 검증/적용
  - ArtifactStore 연동
- 비책임:
  - transport/restart/lifecycle
  - 웹 세션 관리

### 1.4 Web Adapter (`coclai_web`)
- 책임:
  - session/thread tenant 분리
  - SSE event streaming
  - approval REST bridge
- 비책임:
  - 코어 RPC 파서/리듀서 변경
  - 도메인 patch 알고리즘

## 2) 데이터 우선 모델

핵심 상태는 명시적 구조체로 고정합니다.

- 이벤트: `Envelope`
- 런타임 상태: `RuntimeState`, `ThreadState`, `TurnState`, `ItemState`
- 승인 요청: `ServerRequest`, `PendingServerRequest`
- 계측 스냅샷: `RuntimeMetricsSnapshot`
- 웹 세션: `CreateSessionRequest/Response`, `CreateTurnRequest/Response`
- 도메인 patch: `DocPatch`, `DocEdit`, `PatchConflict`

## 3) 순수 변환 vs 부수효과

### 3.1 순수 변환
- `classify_message`
- `extract_ids`
- `reduce` / `reduce_in_place`
- `validate_doc_patch`
- `apply_doc_patch`
- `route_server_request` (approval 라우팅 분기)

### 3.2 부수효과 경계
- child spawn/kill
- stdin/stdout I/O
- timeout scheduling
- file store read/write
- event sink write
- web request/response

원칙:
- 순수 변환은 외부 I/O를 호출하지 않는다.
- 부수효과 함수는 순수 변환 결과를 적용만 한다.

## 4) 런타임 파이프라인

1. transport reader가 JSONL line을 읽는다.
2. dispatcher가 `MsgKind`로 분류한다.
3. pending response resolve / server request queue / notification 처리.
4. `Envelope`를 생성한다.
5. reducer가 상태 스냅샷을 갱신한다.
6. sink(옵션)로 비차단 전달한다.
7. live broadcast로 외부 subscriber에 전달한다.

## 5) 성능 모델

- pending lookup: 평균 O(1)
- reducer map access: 평균 O(1)
- approval lookup: O(1)
- sink 전달: `try_send` 기반 비차단
- metrics: atomic counter 기반 O(1)

핵심 제약:
- 핫패스에서 불필요한 deep copy 금지
- 대기열 포화 시 코어 경로는 멈추지 않고 drop+계측
