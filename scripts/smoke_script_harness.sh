#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

echo "[smoke] gate: schema manifest"
./scripts/check_schema_manifest.sh

echo "[smoke] gate: schema drift (self baseline)"
COCLAI_SCHEMA_DRIFT_MODE=hard COCLAI_SCHEMA_DRIFT_SOURCE=active ./scripts/check_schema_drift.sh

echo "[smoke] gate: doc contract sync"
COCLAI_DOC_SYNC_MODE=hard COCLAI_DOC_SYNC_FAIL_ON_MISMATCH=1 ./scripts/check_doc_contract_sync.sh

echo "[smoke] negative: injected schema drift must fail"
set +e
COCLAI_SCHEMA_DRIFT_MODE=hard COCLAI_SCHEMA_DRIFT_SOURCE=active COCLAI_SCHEMA_DRIFT_INJECT=1 \
  ./scripts/check_schema_drift.sh >/tmp/coclai_smoke_schema_inject.log 2>&1
schema_rc=$?
set -e
if [[ "$schema_rc" -eq 0 ]]; then
  echo "[smoke] expected injected schema drift failure, but command succeeded" >&2
  exit 1
fi

echo "[smoke] negative: injected doc evidence gap must fail"
tmp_doc="$(mktemp)"
tmp_out="$(mktemp)"
cp "$ROOT/Docs/analysis/CONTRACT-MATRIX.md" "$tmp_doc"

set +e
awk '
  BEGIN {
    in_matrix = 0
    changed = 0
  }
  {
    if (index($0, "## Declaration vs Implementation Matrix (T-022 Deliverable)") > 0) {
      in_matrix = 1
    }
    if (in_matrix && !changed && $0 ~ /^\|[[:space:]]*CON-001[[:space:]]*\|/) {
      split($0, cols, "|")
      if (length(cols) >= 5) {
        cols[4] = " - "
        line = cols[1]
        for (i = 2; i <= length(cols); i++) {
          line = line "|" cols[i]
        }
        print line
        changed = 1
        next
      }
    }
    print $0
  }
  END {
    if (!changed) {
      exit 42
    }
  }
' "$tmp_doc" > "$tmp_out"
awk_rc=$?
set -e
if [[ "$awk_rc" -ne 0 ]]; then
  rm -f "$tmp_doc" "$tmp_out"
  echo "[smoke] failed to inject doc evidence gap (awk rc=$awk_rc)" >&2
  exit 1
fi
mv "$tmp_out" "$tmp_doc"

set +e
COCLAI_DOC_SYNC_MODE=hard COCLAI_DOC_CONTRACT_MAP="$tmp_doc" \
  ./scripts/check_doc_contract_sync.sh >/tmp/coclai_smoke_doc_inject.log 2>&1
doc_rc=$?
set -e
rm -f "$tmp_doc"
if [[ "$doc_rc" -eq 0 ]]; then
  echo "[smoke] expected injected doc-sync failure, but command succeeded" >&2
  exit 1
fi

echo "[smoke] script harness passed"
