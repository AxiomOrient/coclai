#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
source "$ROOT/scripts/lib/real_server_retry.sh"

timestamp="$(date -u +"%Y%m%dT%H%M%SZ")"
log_dir="${COCLAI_NIGHTLY_LOG_DIR:-$ROOT/target/qa/nightly_opt_in/$timestamp}"
mkdir -p "$log_dir"

smoke_log="$log_dir/script_smoke.log"
quick_run_log="$log_dir/real_server_quick_run.log"
workflow_log="$log_dir/real_server_workflow.log"
RETRIES="${COCLAI_NIGHTLY_REAL_SERVER_RETRIES:-3}"
BACKOFF_SEC="${COCLAI_NIGHTLY_REAL_SERVER_BACKOFF_SEC:-3}"

echo "[nightly] log_dir=$log_dir"

echo "[nightly] step: script smoke harness"
./scripts/smoke_script_harness.sh 2>&1 | tee "$smoke_log"

run_real_server_gate_with_retries \
  "$RETRIES" \
  "$BACKOFF_SEC" \
  "ergonomic::tests::real_server::quick_run_executes_prompt_against_real_codex_server" \
  "$quick_run_log" \
  "[nightly] step: real-server lane"

run_real_server_gate_with_retries \
  "$RETRIES" \
  "$BACKOFF_SEC" \
  "ergonomic::tests::real_server::workflow_run_executes_prompt_against_real_codex_server" \
  "$workflow_log" \
  "[nightly] step: real-server lane"

echo "[nightly] done"
echo "[nightly] smoke_log=$smoke_log"
echo "[nightly] real_server_quick_run_log=$quick_run_log"
echo "[nightly] real_server_workflow_log=$workflow_log"
