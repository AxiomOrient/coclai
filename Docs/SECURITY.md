# SECURITY

보안 정책을 아래 3계층으로 고정합니다.

## 1) 런타임 기본값

- 기본 approval: `never`
- 기본 sandbox: `readOnly`
- 자동 승인: 금지
- unknown server request: 기본 `auto_decline_unknown=true`

의도:
- 기본 경로에서 파일 변경/명령 실행 같은 고위험 동작을 차단

## 2) 권한 상승(옵션) 규칙

권한 상승은 아래 조건을 모두 만족할 때만 사용:
- 명시적 사용자 요청
- 작업 범위(`cwd`, writable roots) 명시
- 승인 경로(`take_server_request_rx` + `respond_approval_*`) 유지

금지:
- 전역 `danger-full-access`를 기본값으로 두는 설정
- 승인 우회 자동 accept

## 3) 웹 경계 규칙

- tenant/session/thread 교차 접근 금지
- 외부 노출 식별자:
  - 허용: `session_id`, `approval_id`
  - 금지: 내부 `rpc_id`
- SSE/approval 엔드포인트는 동일 tenant 소유권 검증 후 처리

## 4) 로그/데이터 취급

- Envelope/params에는 민감 값이 포함될 수 있으므로 운영 로그에서 최소화
- 장기 저장 시:
  - 보존 기간 제한
  - 필요 시 redaction 적용
  - 접근 제어 정책 적용
