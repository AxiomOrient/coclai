#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"

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

run_real_server_gate_with_retries() {
  local max_attempts="$1"
  local backoff_sec="$2"
  local test_filter="$3"
  local attempt=1
  while (( attempt <= max_attempts )); do
    echo "[release] gate: coclai real-server test '${test_filter}' (attempt ${attempt}/${max_attempts})"
    if cargo test -p coclai "$test_filter" -- --nocapture; then
      return 0
    fi
    if (( attempt == max_attempts )); then
      echo "[release] coclai real-server test '${test_filter}' exhausted retries" >&2
      return 1
    fi
    echo "[release] coclai real-server test '${test_filter}' failed; retrying in ${backoff_sec}s" >&2
    sleep "$backoff_sec"
    attempt=$((attempt + 1))
  done
}

echo "[release] gate: fmt"
cargo fmt --check

echo "[release] gate: clippy"
cargo clippy --workspace --all-targets -- -D warnings

echo "[release] gate: product hygiene"
./scripts/check_product_hygiene.sh

echo "[release] gate: tests"
cargo test --workspace -- \
  --skip ergonomic::tests::real_server::quick_run_executes_prompt_against_real_codex_server \
  --skip ergonomic::tests::real_server::workflow_run_executes_prompt_against_real_codex_server
run_real_server_gate_with_retries \
  "$REAL_SERVER_RETRIES" \
  "$REAL_SERVER_BACKOFF_SEC" \
  "ergonomic::tests::real_server::quick_run_executes_prompt_against_real_codex_server"
run_real_server_gate_with_retries \
  "$REAL_SERVER_RETRIES" \
  "$REAL_SERVER_BACKOFF_SEC" \
  "ergonomic::tests::real_server::workflow_run_executes_prompt_against_real_codex_server"

echo "[release] gate: runtime real-cli contract"
APP_SERVER_CONTRACT=1 APP_SERVER_BIN="${COCLAI_RELEASE_APP_SERVER_BIN:-codex}" \
  cargo test -p coclai_runtime --test contract_real_cli

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
