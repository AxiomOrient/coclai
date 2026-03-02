#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
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
STATE_DIR="${COCLAI_AGENT_STATE_DIR:-$HOME/.coclai/agent}"
PLIST_DIR="$HOME/Library/LaunchAgents"
PLIST_PATH="${PLIST_DIR}/io.coclai.agent.plist"
LABEL="io.coclai.agent"
LAUNCH_TARGET="gui/$(id -u)/${LABEL}"

run_cmd mkdir -p "$BIN_DIR"
run_cmd mkdir -p "$PLIST_DIR"
run_cmd mkdir -p "$STATE_DIR"

run_cmd cargo build --release -p coclai --bin coclai_agent

if [[ -f "$BIN_PATH" ]]; then
  run_cmd cp "$BIN_PATH" "$BACKUP_PATH"
fi
run_cmd cp "${ROOT_DIR}/target/release/coclai_agent" "$BIN_PATH"
run_cmd chmod 755 "$BIN_PATH"

if [[ "$DRY_RUN" == "1" ]]; then
  cat <<EOF
[dry-run] write plist to $PLIST_PATH with:
  ProgramArguments: [$BIN_PATH, serve]
  KeepAlive: true
  RunAtLoad: true
  EnvironmentVariables.COCLAI_AGENT_STATE_DIR=$STATE_DIR
EOF
else
  cat >"$PLIST_PATH" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>${LABEL}</string>
  <key>ProgramArguments</key>
  <array>
    <string>${BIN_PATH}</string>
    <string>serve</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
  <key>StandardOutPath</key>
  <string>${STATE_DIR}/stdout.log</string>
  <key>StandardErrorPath</key>
  <string>${STATE_DIR}/stderr.log</string>
  <key>EnvironmentVariables</key>
  <dict>
    <key>COCLAI_AGENT_STATE_DIR</key>
    <string>${STATE_DIR}</string>
  </dict>
</dict>
</plist>
EOF
fi

if [[ "$SKIP_LAUNCHCTL" == "0" ]]; then
  run_cmd launchctl bootout "gui/$(id -u)" "$PLIST_PATH" || true
  run_cmd launchctl bootstrap "gui/$(id -u)" "$PLIST_PATH"
  run_cmd launchctl kickstart -k "$LAUNCH_TARGET"
fi

echo "installed: $BIN_PATH"
echo "plist: $PLIST_PATH"
echo "backup(if any): $BACKUP_PATH"
