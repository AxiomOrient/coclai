#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"

VIOLATIONS=0

tmp_out="$(mktemp)"
trap 'rm -f "$tmp_out"' EXIT

check_forbidden() {
  local label="$1"
  local pattern="$2"
  local target="$3"

  if [[ ! -d "$target" ]]; then
    echo "[hex-boundary] skip (${label}): missing directory ${target}"
    return
  fi

  if rg -n "$pattern" "$target" >"$tmp_out"; then
    echo "[hex-boundary] violation (${label})"
    cat "$tmp_out"
    VIOLATIONS=1
  else
    echo "[hex-boundary] ok (${label})"
  fi
}

check_forbidden \
  "domain must stay pure" \
  'use[[:space:]]+axum::|use[[:space:]]+tokio::|use[[:space:]]+std::process::Command' \
  "crates/coclai/src/domain"

check_forbidden \
  "application must not depend on adapters" \
  'adapters::|crate::adapters' \
  "crates/coclai/src/application"

if [[ "$VIOLATIONS" -ne 0 ]]; then
  echo "[hex-boundary] FAILED"
  exit 1
fi

echo "[hex-boundary] OK"
