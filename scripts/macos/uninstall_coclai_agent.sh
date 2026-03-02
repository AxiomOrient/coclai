#!/usr/bin/env bash
set -euo pipefail

DRY_RUN=0
KEEP_BINARY=0

for arg in "$@"; do
  case "$arg" in
    --dry-run) DRY_RUN=1 ;;
    --keep-binary) KEEP_BINARY=1 ;;
    *)
      echo "unknown option: $arg" >&2
      echo "usage: $0 [--dry-run] [--keep-binary]" >&2
      exit 2
      ;;
  esac
done

run_cmd() {
  if [[ "$DRY_RUN" == "1" ]]; then
    printf '[dry-run] %q' "$1"
    shift
    for part in "$@"; do
      printf ' %q' "$part"
    done
    printf '\n'
    return 0
  fi
  "$@"
}

BIN_DIR="${COCLAI_AGENT_BIN_DIR:-$HOME/.local/bin}"
BIN_PATH="${BIN_DIR}/coclai-agent"
PLIST_PATH="$HOME/Library/LaunchAgents/io.coclai.agent.plist"

run_cmd launchctl bootout "gui/$(id -u)" "$PLIST_PATH" || true
run_cmd rm -f "$PLIST_PATH"

if [[ "$KEEP_BINARY" == "0" ]]; then
  run_cmd rm -f "$BIN_PATH"
fi

echo "uninstalled plist: $PLIST_PATH"
if [[ "$KEEP_BINARY" == "0" ]]; then
  echo "removed binary: $BIN_PATH"
else
  echo "kept binary: $BIN_PATH"
fi
