#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"
source "$ROOT_DIR/scripts/lib/real_server_retry.sh"

INCLUDE_PERF="${COCLAI_RELEASE_INCLUDE_PERF:-0}"
INCLUDE_NIGHTLY="${COCLAI_RELEASE_INCLUDE_NIGHTLY:-0}"
REAL_SERVER_RETRIES="${COCLAI_RELEASE_REAL_SERVER_RETRIES:-3}"
REAL_SERVER_BACKOFF_SEC="${COCLAI_RELEASE_REAL_SERVER_BACKOFF_SEC:-3}"

case "$INCLUDE_PERF" in
  0|1) ;;
  *)
    echo "[release] invalid COCLAI_RELEASE_INCLUDE_PERF=$INCLUDE_PERF (allowed: 0|1)" >&2
    exit 2
    ;;
esac

case "$INCLUDE_NIGHTLY" in
  0|1) ;;
  *)
    echo "[release] invalid COCLAI_RELEASE_INCLUDE_NIGHTLY=$INCLUDE_NIGHTLY (allowed: 0|1)" >&2
    exit 2
    ;;
esac

case "$REAL_SERVER_RETRIES" in
  ''|*[!0-9]*)
    echo "[release] invalid COCLAI_RELEASE_REAL_SERVER_RETRIES=$REAL_SERVER_RETRIES (allowed: integer >= 1)" >&2
    exit 2
    ;;
esac

if [ "$REAL_SERVER_RETRIES" -lt 1 ]; then
  echo "[release] invalid COCLAI_RELEASE_REAL_SERVER_RETRIES=$REAL_SERVER_RETRIES (allowed: integer >= 1)" >&2
  exit 2
fi

case "$REAL_SERVER_BACKOFF_SEC" in
  ''|*[!0-9]*)
    echo "[release] invalid COCLAI_RELEASE_REAL_SERVER_BACKOFF_SEC=$REAL_SERVER_BACKOFF_SEC (allowed: non-negative integer)" >&2
    exit 2
    ;;
esac

echo "[release] gate: fmt"
cargo fmt --check

echo "[release] gate: clippy"
cargo clippy --workspace --all-targets -- -D warnings

echo "[release] gate: hexagonal boundaries"
./scripts/check_hexagonal_boundaries.sh

echo "[release] gate: product hygiene"
COCLAI_HYGIENE_SKIP_CLIPPY=1 ./scripts/check_product_hygiene.sh

echo "[release] gate: tests"
cargo test --workspace -- \
  --skip ergonomic::tests::real_server::quick_run_executes_prompt_against_real_codex_server \
  --skip ergonomic::tests::real_server::workflow_run_executes_prompt_against_real_codex_server
run_real_server_gate_with_retries \
  "$REAL_SERVER_RETRIES" \
  "$REAL_SERVER_BACKOFF_SEC" \
  "ergonomic::tests::real_server::quick_run_executes_prompt_against_real_codex_server" \
  "" \
  "[release] gate: coclai real-server test"
run_real_server_gate_with_retries \
  "$REAL_SERVER_RETRIES" \
  "$REAL_SERVER_BACKOFF_SEC" \
  "ergonomic::tests::real_server::workflow_run_executes_prompt_against_real_codex_server" \
  "" \
  "[release] gate: coclai real-server test"

echo "[release] gate: agent go/no-go"
COCLAI_AGENT_GO_NO_GO_MODE=ops-only ./scripts/release_agent_go_no_go.sh

echo "[release] gate: schema drift"
COCLAI_SCHEMA_DRIFT_MODE=hard COCLAI_SCHEMA_DRIFT_SOURCE="${COCLAI_RELEASE_SCHEMA_DRIFT_SOURCE:-codex}" ./scripts/check_schema_drift.sh

echo "[release] gate: schema manifest"
./scripts/check_schema_manifest.sh

echo "[release] gate: doc contract sync"
COCLAI_DOC_SYNC_MODE=hard COCLAI_DOC_SYNC_FAIL_ON_MISMATCH=1 ./scripts/check_doc_contract_sync.sh

if [[ "$INCLUDE_PERF" == "1" ]]; then
  echo "[release] gate: micro bench"
  ./scripts/run_micro_bench.sh
else
  echo "[release] gate: micro bench (skipped; set COCLAI_RELEASE_INCLUDE_PERF=1)"
fi

if [[ "$INCLUDE_NIGHTLY" == "1" ]]; then
  echo "[release] gate: nightly opt-in"
  ./scripts/run_nightly_opt_in_gate.sh
else
  echo "[release] gate: nightly opt-in (skipped; set COCLAI_RELEASE_INCLUDE_NIGHTLY=1)"
fi

echo "[release] preflight passed"
