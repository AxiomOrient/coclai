#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"

PKG="${CODEX_RUNTIME_PKG:-codex-runtime}"

echo "[security] gate: approval routing policy"
cargo test -p "$PKG" runtime::approvals::tests:: -- --nocapture

echo "[security] gate: unknown server request auto-decline"
cargo test -p "$PKG" runtime::core::tests::server_requests::validation_and_unknown::unknown_server_request_is_auto_declined -- --nocapture

echo "[security] gate: privileged sandbox validation"
cargo test -p "$PKG" runtime::api::tests::thread_api::thread_start_rejects_privileged_sandbox_ -- --nocapture
cargo test -p "$PKG" runtime::api::tests::thread_api::turn_start_rejects_privileged_sandbox_ -- --nocapture

echo "[security] gate: web approval bridge"
cargo test -p "$PKG" adapters::web::tests::approvals:: -- --nocapture
cargo test -p "$PKG" adapters::web::tests::approval_boundaries:: -- --nocapture

echo "[security] gate: passed"
