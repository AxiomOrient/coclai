# Evidence Map (Consolidated)

이 문서는 삭제된 세부 phase 문서(`phase-0..6`, `legacy-prune-preflight`)의 증거를 운영용 최소 단위로 재매핑한 canonical index다.

## phase-0-baseline-evidence

- Covered tasks: `T-010`, `T-011`, `T-012`
- Frozen findings:
  - Upstream client-request method set: `42`
  - Wrapper first-class + pass-through gap baseline: `10 + 32`
- Artifacts:
  - `Cargo.toml` (workspace/crates baseline)
  - `README.md` (scope/contracts baseline)
  - `SCHEMAS/app-server/active/*` (schema baseline)
  - `crates/coclai/src/appserver.rs` (upstream method surface bridge)
- Deterministic commands:
  - `rg --files | wc -l`
  - `cargo metadata --format-version 1 --no-deps`
  - `rg -n "thread/|turn/|model/|skills/" crates/coclai/src/appserver.rs`

## legacy-prune-evidence

- Covered task: `T-015`
- Artifacts:
  - `Docs/TASKS.md` (canonical path migration complete)
  - `Docs/analysis/EVIDENCE-MAP.md` (legacy evidence consolidation)
- Deterministic commands:
  - `find Docs -type f -name '*.md' | sort`
  - `rg -n "docs/" Docs README.md scripts .github || true`

## phase-1-contract-evidence

- Covered tasks: `T-020`, `T-021`, `T-022`
- Artifacts:
  - `Docs/analysis/CONTRACT-MATRIX.md` (declaration vs implementation matrix)
  - `README.md`, `Docs/ARCHITECTURE.md`, `Docs/CORE_API.md`, `Docs/SCHEMA_AND_CONTRACT.md`, `Docs/SECURITY.md`
- Deterministic commands:
  - `./scripts/check_doc_contract_sync.sh`
  - `rg -n "^## Declaration vs Implementation Matrix" Docs/analysis/CONTRACT-MATRIX.md`

## phase-2-runtime-evidence

- Covered tasks: `T-030`, `T-031`, `T-032`, `T-033`, `T-034`
- Frozen findings:
  - Core hotspot LOC: `api.rs=1906`, `client.rs=1032`, `runtime.rs=664`
  - 목표 의존성 위반 edge(`runtime -> artifact|web`)는 baseline 기준 `0`
- Artifacts:
  - `crates/coclai_runtime/src/runtime.rs`
  - `crates/coclai_runtime/src/runtime/lifecycle.rs`
  - `crates/coclai_runtime/src/runtime/supervisor.rs`
  - `crates/coclai_runtime/src/api.rs`, `crates/coclai_runtime/src/api/{models,ops,flow}.rs`
  - `crates/coclai_runtime/src/client.rs`, `crates/coclai_runtime/src/client/{config,session,profile,compat_guard}.rs`
- Deterministic commands:
  - `cargo test -q -p coclai_runtime --lib`

## phase-3-usability-evidence

- Covered tasks: `T-040`, `T-041`, `T-042`
- Frozen findings:
  - Upstream method set `42`, first-class surfaced `10`, pass-through callable `32`
  - Example LOC ratio: `workflow ≈ 2.9x quick_run`
- Artifacts:
  - `crates/coclai/src/appserver.rs`
  - `crates/coclai/src/ergonomic.rs`
  - `crates/coclai_runtime/src/errors.rs`
- Deterministic commands:
  - `cargo test -q -p coclai --lib`

## phase-4-adapter-evidence

- Covered tasks: `T-050`, `T-051`, `T-052`
- Artifacts:
  - `crates/coclai_artifact/src/{lib.rs,adapter.rs,store.rs,patch.rs,task.rs}`
  - `crates/coclai_web/src/{lib.rs,adapter.rs,state.rs,wire.rs}`
  - `crates/coclai_plugin_core/src/lib.rs`
- Deterministic commands:
  - `cargo test -q -p coclai_artifact --lib`
  - `cargo test -q -p coclai_web --lib`

## phase-5-quality-evidence

- Covered tasks: `T-060`, `T-061`, `T-062`
- Frozen findings:
  - Drift snapshot: `active=85`, `generated-now=142`
  - Diff summary: `missing in active=59`, `only in active=2`, `hash-diff(common)=41`
  - Coverage marker baseline: total test marker `203`, scripted release/schema pipeline risk level `H0`
- Artifacts:
  - `scripts/release_preflight.sh`
  - `scripts/check_schema_manifest.sh`
  - `scripts/check_schema_drift.sh`
  - `scripts/check_doc_contract_sync.sh`
  - `scripts/run_micro_bench.sh`
- Deterministic commands:
  - `./scripts/release_preflight.sh`
  - `./scripts/smoke_script_harness.sh`

## phase-6-options-evidence

- Covered tasks: `T-070`, `T-071`, `T-072`
- Frozen findings:
  - Option utility: `A=3.20`, `B=2.70`, `C=4.15` (`Option C` 채택)
  - Atomic sequence: `R1 -> R2 -> R3 -> R4 -> R5 -> R6`
- Artifacts:
  - `Docs/analysis/FINAL-ANALYSIS-REPORT.md`
  - `Docs/analysis/NEXT-ACTIONS.md`
  - `Docs/IMPLEMENTATION-PLAN.md`
- Deterministic commands:
  - `rg -n "Option C|R1 -> R2 -> R3 -> R4 -> R5 -> R6" Docs/analysis/FINAL-ANALYSIS-REPORT.md`
  - `rg -n "I-0|D-0|VerifyGate|Rollback" Docs/analysis/NEXT-ACTIONS.md`
