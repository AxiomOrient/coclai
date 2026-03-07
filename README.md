# coclai

`coclai`는 로컬 `codex app-server`를 Rust에서 안전하게 감싸는 라이브러리입니다.

## 핵심
1. 런타임 라이프사이클 표준화 (`connect -> run/setup -> ask -> close -> shutdown`)
2. JSON-RPC 계약 검증
3. opt-in 실서버 게이트를 포함한 최소 릴리즈 검증

## 저장소 구조
- `crates/coclai`: 단일 패키지
- `crates/coclai/src/runtime`: 런타임/JSON-RPC/상태/승인 라우팅
- `crates/coclai/src/domain/artifact`: artifact 도메인
- `crates/coclai/src/adapters/web`: web 세션/approval 어댑터
- `docs`: 유지 중인 공개 문서
- `scripts`: 릴리즈/품질 게이트 스크립트

## 설치
```toml
[dependencies]
coclai = { path = "crates/coclai" }
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

## 빠른 시작
```rust
use coclai::quick_run;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = quick_run("/ABS/PATH/WORKDIR", "핵심 3줄 요약").await?;
    println!("{}", out.assistant_text);
    Ok(())
}
```

## 기본 검증
```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
./scripts/check_product_hygiene.sh
./scripts/check_security_gate.sh
./scripts/check_blocker_regressions.sh
cargo test --workspace
```

## 릴리즈 preflight
```bash
./scripts/release_preflight.sh
```

실서버 opt-in 테스트를 포함하려면:
```bash
COCLAI_REAL_SERVER_APPROVED=1 \
COCLAI_RELEASE_INCLUDE_REAL_SERVER=1 \
./scripts/release_preflight.sh
```

## 문서 맵
- `docs/CORE_API.md`: 공개 API, 런타임 계약, opt-in 실서버 게이트
- `docs/TEST_TREE.md`: 테스트 레이어 구조와 실행 기준

## 실서버 검증 범위
- 기본 파이프라인은 deterministic 게이트만 실행
- opt-in 실서버 게이트는 `quick_run`, `workflow.run`, attachment 포함 `quick_run_with_profile`, session/resume, low-level `AppServer`, approval roundtrip까지 검증
- `requestUserInput`, dynamic tool-call live 검증은 현재 wrapper 표면 제약 때문에 mock/unit 경로만 유지

## 라이선스
MIT
