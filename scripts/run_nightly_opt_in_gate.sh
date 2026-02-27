#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

timestamp="$(date -u +"%Y%m%dT%H%M%SZ")"
log_dir="${COCLAI_NIGHTLY_LOG_DIR:-$ROOT/target/qa/nightly_opt_in/$timestamp}"
mkdir -p "$log_dir"

smoke_log="$log_dir/script_smoke.log"
real_cli_log="$log_dir/real_cli_contract.log"

echo "[nightly] log_dir=$log_dir"

echo "[nightly] step: script smoke harness"
./scripts/smoke_script_harness.sh 2>&1 | tee "$smoke_log"

echo "[nightly] step: real CLI contract (opt-in)"
APP_SERVER_CONTRACT=1 cargo test -p coclai_runtime --test contract_real_cli -- --nocapture \
  2>&1 | tee "$real_cli_log"

skip_phrase="skipping real CLI contract test; set APP_SERVER_CONTRACT=1 to enable this test"
if command -v rg >/dev/null 2>&1; then
  rg -n "$skip_phrase" "$real_cli_log" >/dev/null && skipped=1 || skipped=0
else
  grep -n "$skip_phrase" "$real_cli_log" >/dev/null && skipped=1 || skipped=0
fi
if [[ "$skipped" -eq 1 ]]; then
  echo "[nightly] real CLI contract lane was skipped unexpectedly" >&2
  exit 1
fi

echo "[nightly] done"
echo "[nightly] smoke_log=$smoke_log"
echo "[nightly] real_cli_log=$real_cli_log"
