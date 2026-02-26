#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SCHEMA_DIR="$ROOT/SCHEMAS/app-server/active/json-schema"
MANIFEST="$ROOT/SCHEMAS/app-server/active/manifest.sha256"

if [[ ! -d "$SCHEMA_DIR" ]]; then
  echo "schema dir not found: $SCHEMA_DIR" >&2
  exit 1
fi

cd "$SCHEMA_DIR"

hash_file() {
  local path="$1"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$path"
  else
    shasum -a 256 "$path"
  fi
}

: > "$MANIFEST"
while IFS= read -r -d '' file; do
  hash_file "$file" >> "$MANIFEST"
done < <(find . -type f -print0 | sort -z)

echo "wrote $MANIFEST"
