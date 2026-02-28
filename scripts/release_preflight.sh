#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"

echo "[release] gate: fmt"
cargo fmt --check

echo "[release] gate: clippy"
cargo clippy --workspace --all-targets -- -D warnings

echo "[release] gate: product hygiene"
./scripts/check_product_hygiene.sh

echo "[release] gate: tests"
cargo test --workspace

echo "[release] gate: schema drift"
COCLAI_SCHEMA_DRIFT_MODE=hard COCLAI_SCHEMA_DRIFT_SOURCE="${COCLAI_RELEASE_SCHEMA_DRIFT_SOURCE:-codex}" ./scripts/check_schema_drift.sh

echo "[release] gate: schema manifest"
./scripts/check_schema_manifest.sh

echo "[release] gate: doc contract sync"
COCLAI_DOC_SYNC_MODE=hard COCLAI_DOC_SYNC_FAIL_ON_MISMATCH=1 ./scripts/check_doc_contract_sync.sh

echo "[release] preflight passed"
