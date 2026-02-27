#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"

echo "[hygiene] gate: unfinished markers in product sources"
MARKERS='todo!\(|unimplemented!\(|\b(TODO|FIXME|TBD|not implemented|미구현)\b'
if rg -n \
  --glob '**/*.rs' \
  --glob '!**/tests.rs' \
  --glob '!**/*_tests.rs' \
  --glob '!**/tests/**' \
  "$MARKERS" \
  crates/*/src; then
  echo "[hygiene] found unfinished markers in product sources"
  exit 1
fi

echo "[hygiene] gate: panic/unwrap/expect forbidden in production targets"
cargo clippy --workspace --lib --bins --examples -- \
  -D warnings \
  -D clippy::panic \
  -D clippy::unwrap_used \
  -D clippy::expect_used

echo "[hygiene] passed"
