#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"

PKG="${CODEX_RUNTIME_PKG:-codex-runtime}"

BLOCKER_TESTS=(
  "runtime::api::tests::run_prompt::run_prompt_lagged_thread_read_respects_absolute_deadline"
  "runtime::core::tests::core_lifecycle::call_raw_abort_cleans_pending_rpc_entry"
  "runtime::core::tests::server_requests::lifecycle_guards::full_server_request_queue_does_not_stall_dispatcher"
)

echo "[blocker] gate: required regression tests are present"
LIST_OUTPUT="$(cargo test -p "$PKG" -- --list)"
for test_name in "${BLOCKER_TESTS[@]}"; do
  if ! printf '%s\n' "$LIST_OUTPUT" | grep -Fqx "${test_name}: test"; then
    echo "[blocker] missing required blocker regression test: ${test_name}" >&2
    exit 1
  fi
done

echo "[blocker] gate: required regression tests execute and pass"
for test_name in "${BLOCKER_TESTS[@]}"; do
  echo "[blocker] running ${test_name}"
  cargo test -p "$PKG" "$test_name" -- --exact --nocapture
done

echo "[blocker] passed"
