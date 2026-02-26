#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"

echo "[release] gate: fmt"
cargo fmt --check

echo "[release] gate: clippy"
cargo clippy --workspace --all-targets -- -D warnings

echo "[release] gate: tests"
cargo test --workspace

echo "[release] gate: schema manifest"
./scripts/check_schema_manifest.sh

echo "[release] preflight passed"
