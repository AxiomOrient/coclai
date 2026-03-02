#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"

STATE_DIR="${COCLAI_AGENT_STATE_DIR:-$ROOT_DIR/target/qa/agent-go-no-go/state}"
MODE="${COCLAI_AGENT_GO_NO_GO_MODE:-full}" # full|ops-only

case "$MODE" in
  full|ops-only) ;;
  *)
    echo "[agent-go-no-go] invalid mode: $MODE (allowed: full|ops-only)" >&2
    exit 2
    ;;
esac

mkdir -p "$STATE_DIR"

if [[ "$MODE" == "full" ]]; then
  echo "[agent-go-no-go] gate: hexagonal boundary static check"
  bash scripts/check_hexagonal_boundaries.sh

  echo "[agent-go-no-go] gate: cargo check"
  cargo check -p coclai

  echo "[agent-go-no-go] gate: clippy -D warnings"
  cargo clippy -p coclai --all-targets -- -D warnings

  echo "[agent-go-no-go] gate: coclai tests (skip real-server flaky lanes)"
  cargo test -p coclai --lib --tests -- \
    --skip ergonomic::tests::real_server::quick_run_executes_prompt_against_real_codex_server \
    --skip ergonomic::tests::real_server::workflow_run_executes_prompt_against_real_codex_server

  echo "[agent-go-no-go] gate: doc contract sync"
  COCLAI_DOC_SYNC_MODE=hard COCLAI_DOC_SYNC_FAIL_ON_MISMATCH=1 \
    bash scripts/check_doc_contract_sync.sh
fi

echo "[agent-go-no-go] gate: process lifecycle start/status/stop"
COCLAI_AGENT_STATE_DIR="$STATE_DIR" cargo run -q -p coclai --bin coclai_agent -- start >/dev/null
COCLAI_AGENT_STATE_DIR="$STATE_DIR" cargo run -q -p coclai --bin coclai_agent -- status >/dev/null
COCLAI_AGENT_STATE_DIR="$STATE_DIR" cargo run -q -p coclai --bin coclai_agent -- stop >/dev/null
COCLAI_AGENT_STATE_DIR="$STATE_DIR" cargo run -q -p coclai --bin coclai_agent -- status >/dev/null

echo "[agent-go-no-go] gate: cli option guard"
if COCLAI_AGENT_STATE_DIR="$STATE_DIR" cargo run -q -p coclai --bin coclai_agent -- invoke quick_run --foo >/dev/null 2>&1; then
  echo "[agent-go-no-go] expected option-guard failure did not occur" >&2
  exit 1
fi

echo "[agent-go-no-go] gate: network ingress security guard"
COCLAI_AGENT_TOKEN="go-no-go-token" \
COCLAI_AGENT_STATE_DIR="$STATE_DIR" \
  cargo run -q -p coclai --bin coclai_agent -- \
  invoke system/health --ingress http --caller 127.0.0.1:39000 --token go-no-go-token >/dev/null

if COCLAI_AGENT_TOKEN="go-no-go-token" \
  COCLAI_AGENT_STATE_DIR="$STATE_DIR" \
  cargo run -q -p coclai --bin coclai_agent -- \
  invoke system/health --ingress http --caller 127.0.0.1:39000 >/dev/null 2>&1; then
  echo "[agent-go-no-go] expected missing-token failure did not occur" >&2
  exit 1
fi

if COCLAI_AGENT_TOKEN="go-no-go-token" \
  COCLAI_AGENT_STATE_DIR="$STATE_DIR" \
  cargo run -q -p coclai --bin coclai_agent -- \
  invoke system/health --ingress http --caller 10.10.0.1:39000 --token go-no-go-token >/dev/null 2>&1; then
  echo "[agent-go-no-go] expected non-loopback rejection did not occur" >&2
  exit 1
fi

echo "[agent-go-no-go] gate: macOS packaging scripts syntax + dry-run"
bash -n scripts/macos/install_coclai_agent.sh scripts/macos/uninstall_coclai_agent.sh scripts/macos/rollback_coclai_agent.sh
scripts/macos/install_coclai_agent.sh --dry-run --skip-launchctl >/dev/null
scripts/macos/uninstall_coclai_agent.sh --dry-run --keep-binary >/dev/null
scripts/macos/rollback_coclai_agent.sh --dry-run --skip-launchctl >/dev/null

echo "[agent-go-no-go] PASSED (mode=$MODE)"
