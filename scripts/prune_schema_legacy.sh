#!/usr/bin/env bash
set -euo pipefail

TARGET_DIR="${1:-}"
if [[ -z "$TARGET_DIR" ]]; then
  echo "usage: $0 <schema-dir>" >&2
  exit 1
fi
if [[ ! -d "$TARGET_DIR" ]]; then
  echo "schema dir not found: $TARGET_DIR" >&2
  exit 1
fi

rm -rf "$TARGET_DIR/v1"
rm -f "$TARGET_DIR/app_server_protocol.schemas.json"
rm -f "$TARGET_DIR/ApplyPatchApprovalParams.json"
rm -f "$TARGET_DIR/ApplyPatchApprovalResponse.json"
rm -f "$TARGET_DIR/ClientNotification.json"
rm -f "$TARGET_DIR/ClientRequest.json"
rm -f "$TARGET_DIR/EventMsg.json"
rm -f "$TARGET_DIR/ExecCommandApprovalParams.json"
rm -f "$TARGET_DIR/ExecCommandApprovalResponse.json"
rm -f "$TARGET_DIR/FuzzyFileSearchParams.json"
rm -f "$TARGET_DIR/FuzzyFileSearchResponse.json"
rm -f "$TARGET_DIR/JSONRPCError.json"
rm -f "$TARGET_DIR/JSONRPCErrorError.json"
rm -f "$TARGET_DIR/JSONRPCMessage.json"
rm -f "$TARGET_DIR/JSONRPCNotification.json"
rm -f "$TARGET_DIR/JSONRPCRequest.json"
rm -f "$TARGET_DIR/JSONRPCResponse.json"
rm -f "$TARGET_DIR/RequestId.json"
rm -f "$TARGET_DIR/ServerNotification.json"
rm -f "$TARGET_DIR/ServerRequest.json"

