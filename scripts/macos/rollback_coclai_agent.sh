#!/usr/bin/env bash
set -euo pipefail

DRY_RUN=0
SKIP_LAUNCHCTL=0

for arg in "$@"; do
  case "$arg" in
    --dry-run) DRY_RUN=1 ;;
    --skip-launchctl) SKIP_LAUNCHCTL=1 ;;
    *)
      echo "unknown option: $arg" >&2
      echo "usage: $0 [--dry-run] [--skip-launchctl]" >&2
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
BACKUP_PATH="${BIN_PATH}.bak"
PLIST_PATH="$HOME/Library/LaunchAgents/io.coclai.agent.plist"
LABEL="io.coclai.agent"
LAUNCH_TARGET="gui/$(id -u)/${LABEL}"

if [[ ! -f "$BACKUP_PATH" && "$DRY_RUN" == "0" ]]; then
  echo "backup not found: $BACKUP_PATH" >&2
  exit 1
fi

run_cmd cp "$BACKUP_PATH" "$BIN_PATH"
run_cmd chmod 755 "$BIN_PATH"

if [[ "$SKIP_LAUNCHCTL" == "0" ]]; then
  run_cmd launchctl bootout "gui/$(id -u)" "$PLIST_PATH" || true
  run_cmd launchctl bootstrap "gui/$(id -u)" "$PLIST_PATH"
  run_cmd launchctl kickstart -k "$LAUNCH_TARGET"
fi

echo "rolled back binary from: $BACKUP_PATH"
