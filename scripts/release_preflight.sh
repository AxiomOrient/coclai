#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"

PKG="${COCLAI_PKG:-coclai}"
INCLUDE_REAL_SERVER="${COCLAI_RELEASE_INCLUDE_REAL_SERVER:-0}"
REAL_SERVER_RETRIES="${COCLAI_RELEASE_REAL_SERVER_RETRIES:-3}"
REAL_SERVER_BACKOFF_SEC="${COCLAI_RELEASE_REAL_SERVER_BACKOFF_SEC:-3}"
REAL_SERVER_APPROVED="${COCLAI_REAL_SERVER_APPROVED:-0}"

case "$INCLUDE_REAL_SERVER" in
  0|1) ;;
  *)
    echo "[release] invalid COCLAI_RELEASE_INCLUDE_REAL_SERVER=$INCLUDE_REAL_SERVER (allowed: 0|1)" >&2
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

case "$REAL_SERVER_APPROVED" in
  0|1) ;;
  *)
    echo "[release] invalid COCLAI_REAL_SERVER_APPROVED=$REAL_SERVER_APPROVED (allowed: 0|1)" >&2
    exit 2
    ;;
esac

if ! command -v python3 >/dev/null 2>&1; then
  echo "[release] python3 is required for mock-process runtime tests (runtime/api/core fixtures)" >&2
  exit 2
fi

run_real_server_gate_with_retries() {
  local max_attempts="$1"
  local backoff_sec="$2"
  local test_filter="$3"
  local attempt=1
  while (( attempt <= max_attempts )); do
    echo "[release] gate: ${PKG} real-server test '${test_filter}' (attempt ${attempt}/${max_attempts})"
    if cargo test -p "$PKG" "$test_filter" -- --ignored --nocapture; then
      return 0
    fi
    if (( attempt == max_attempts )); then
      echo "[release] ${PKG} real-server test '${test_filter}' exhausted retries" >&2
      return 1
    fi
    echo "[release] ${PKG} real-server test '${test_filter}' failed; retrying in ${backoff_sec}s" >&2
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

echo "[release] gate: security"
./scripts/check_security_gate.sh

echo "[release] gate: blocker regressions"
./scripts/check_blocker_regressions.sh

echo "[release] gate: tests"
cargo test --workspace

if [[ "$INCLUDE_REAL_SERVER" == "1" ]]; then
  if [[ "$REAL_SERVER_APPROVED" != "1" ]]; then
    echo "[release] refusing real-server gate: set COCLAI_REAL_SERVER_APPROVED=1 after explicit operator approval" >&2
    exit 2
  fi
  echo "[release] gate: real-server contract"
  run_real_server_gate_with_retries \
    "$REAL_SERVER_RETRIES" \
    "$REAL_SERVER_BACKOFF_SEC" \
    "ergonomic::tests::real_server::"
else
  echo "[release] gate: real-server contract (skipped; set COCLAI_RELEASE_INCLUDE_REAL_SERVER=1)"
fi

echo "[release] preflight passed"
