#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ACTIVE_DIR="$ROOT/SCHEMAS/app-server/active/json-schema"
MODE="${COCLAI_SCHEMA_DRIFT_MODE:-soft}" # hard|soft|off
SOURCE="${COCLAI_SCHEMA_DRIFT_SOURCE:-codex}" # codex|active
INJECT="${COCLAI_SCHEMA_DRIFT_INJECT:-0}" # 1 => inject deterministic drift (for tests)

case "$MODE" in
  hard|soft|off) ;;
  *)
    echo "[schema-drift] invalid mode: $MODE (allowed: hard|soft|off)" >&2
    exit 2
    ;;
esac

if [[ "$MODE" == "off" ]]; then
  echo "[schema-drift] skipped (mode=off)"
  exit 0
fi

if [[ ! -d "$ACTIVE_DIR" ]]; then
  echo "[schema-drift] active schema dir not found: $ACTIVE_DIR" >&2
  exit 1
fi

if [[ "$SOURCE" != "codex" && "$SOURCE" != "active" ]]; then
  echo "[schema-drift] invalid source: $SOURCE (allowed: codex|active)" >&2
  exit 2
fi

hash_file() {
  local path="$1"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$path"
  else
    shasum -a 256 "$path"
  fi
}

write_manifest() {
  local schema_dir="$1"
  local out="$2"
  (
    cd "$schema_dir"
    : > "$out"
    while IFS= read -r -d '' file; do
      hash_file "$file" >> "$out"
    done < <(find . -type f -print0 | sort -z)
  )
}

to_index() {
  local manifest="$1"
  local out="$2"
  awk '{
    hash=$1
    $1=""
    sub(/^[[:space:]]+/, "", $0)
    print $0 "\t" hash
  }' "$manifest" | LC_ALL=C sort > "$out"
}

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

GEN_DIR="$TMP_DIR/generated"
ACT_DIR="$TMP_DIR/active"
mkdir -p "$GEN_DIR" "$ACT_DIR"

cp -R "$ACTIVE_DIR/." "$ACT_DIR/"
bash "$ROOT/scripts/prune_schema_legacy.sh" "$ACT_DIR"

if [[ "$SOURCE" == "codex" ]]; then
  codex app-server generate-json-schema --out "$GEN_DIR"
else
  cp -R "$ACT_DIR/." "$GEN_DIR/"
fi
bash "$ROOT/scripts/prune_schema_legacy.sh" "$GEN_DIR"

if [[ "$INJECT" == "1" ]]; then
  printf '{ "injected": true }\n' > "$GEN_DIR/__drift_injected__.json"
fi

GEN_MANIFEST="$TMP_DIR/generated.sha256"
ACT_MANIFEST="$TMP_DIR/active.sha256"
GEN_INDEX="$TMP_DIR/generated.tsv"
ACT_INDEX="$TMP_DIR/active.tsv"
GEN_PATHS="$TMP_DIR/generated.paths"
ACT_PATHS="$TMP_DIR/active.paths"
MISSING="$TMP_DIR/missing.paths"
EXTRA="$TMP_DIR/extra.paths"
HASH_DIFF="$TMP_DIR/hash-diff.tsv"

write_manifest "$GEN_DIR" "$GEN_MANIFEST"
write_manifest "$ACT_DIR" "$ACT_MANIFEST"
to_index "$GEN_MANIFEST" "$GEN_INDEX"
to_index "$ACT_MANIFEST" "$ACT_INDEX"
cut -f1 "$GEN_INDEX" > "$GEN_PATHS"
cut -f1 "$ACT_INDEX" > "$ACT_PATHS"
comm -23 "$GEN_PATHS" "$ACT_PATHS" > "$MISSING"
comm -13 "$GEN_PATHS" "$ACT_PATHS" > "$EXTRA"
join -t $'\t' -j 1 "$GEN_INDEX" "$ACT_INDEX" \
  | awk -F'\t' '$2 != $3 {print $1 "\t" $2 "\t" $3}' > "$HASH_DIFF"

missing_count="$(wc -l < "$MISSING" | tr -d ' ')"
extra_count="$(wc -l < "$EXTRA" | tr -d ' ')"
hash_diff_count="$(wc -l < "$HASH_DIFF" | tr -d ' ')"

if [[ "$missing_count" == "0" && "$extra_count" == "0" && "$hash_diff_count" == "0" ]]; then
  echo "[schema-drift] OK (mode=$MODE, source=$SOURCE)"
  exit 0
fi

echo "[schema-drift] drift detected (mode=$MODE, source=$SOURCE): missing=$missing_count extra=$extra_count hash_diff=$hash_diff_count" >&2
if [[ "$missing_count" != "0" ]]; then
  echo "[schema-drift] missing in active (first 20):" >&2
  head -n 20 "$MISSING" >&2
fi
if [[ "$extra_count" != "0" ]]; then
  echo "[schema-drift] only in active (first 20):" >&2
  head -n 20 "$EXTRA" >&2
fi
if [[ "$hash_diff_count" != "0" ]]; then
  echo "[schema-drift] hash diff (first 20):" >&2
  head -n 20 "$HASH_DIFF" | awk -F'\t' '{printf "%s\n", $1}' >&2
fi

if [[ "$MODE" == "hard" ]]; then
  exit 1
fi

echo "[schema-drift] warning only (soft mode)" >&2
exit 0
