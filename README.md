# coclai

`coclai`는 로컬 `codex app-server`를 Rust에서 안전하게 감싸는 라이브러리입니다.

## 핵심
1. 런타임 라이프사이클 표준화 (`connect -> run/setup -> ask -> close -> shutdown`)
2. JSON-RPC 계약 검증
3. 최소 릴리즈 게이트(포맷/린트/보안/테스트)

## 저장소 구조
- `crates/coclai`: 단일 패키지
- `crates/coclai/src/runtime`: 런타임/JSON-RPC/상태/승인 라우팅
- `crates/coclai/src/domain/artifact`: artifact 도메인
- `crates/coclai/src/adapters/web`: web 세션/approval 어댑터
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
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
./scripts/check_product_hygiene.sh
./scripts/check_security_gate.sh
cargo test --workspace
```

## 릴리즈 preflight
```bash
./scripts/release_preflight.sh
```

실서버 opt-in 테스트를 포함하려면:
```bash
COCLAI_RELEASE_INCLUDE_REAL_SERVER=1 ./scripts/release_preflight.sh
```

## 문서 맵
- `docs/CORE_API.md`
- `docs/IMPLEMENTATION-PLAN.md`
- `docs/TASKS.md`

## 라이선스
MIT
