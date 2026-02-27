#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DOC_MAP="${COCLAI_DOC_CONTRACT_MAP:-$ROOT/Docs/analysis/CONTRACT-MATRIX.md}"
MODE="${COCLAI_DOC_SYNC_MODE:-hard}" # hard|soft|off
FAIL_ON_MISMATCH="${COCLAI_DOC_SYNC_FAIL_ON_MISMATCH:-0}" # 1 => fail when mismatch verdict exists

case "$MODE" in
  hard|soft|off) ;;
  *)
    echo "[doc-sync] invalid mode: $MODE (allowed: hard|soft|off)" >&2
    exit 2
    ;;
esac

case "$FAIL_ON_MISMATCH" in
  0|1) ;;
  *)
    echo "[doc-sync] invalid COCLAI_DOC_SYNC_FAIL_ON_MISMATCH=$FAIL_ON_MISMATCH (allowed: 0|1)" >&2
    exit 2
    ;;
esac

if [[ "$MODE" == "off" ]]; then
  echo "[doc-sync] skipped (mode=off)"
  exit 0
fi

if [[ ! -f "$DOC_MAP" ]]; then
  echo "[doc-sync] doc contract map not found: $DOC_MAP" >&2
  if [[ "$MODE" == "soft" ]]; then
    echo "[doc-sync] warning only (soft mode)" >&2
    exit 0
  fi
  exit 1
fi

set +e
output="$(python3 - "$DOC_MAP" "$FAIL_ON_MISMATCH" <<'PY'
from __future__ import annotations

import collections
import pathlib
import re
import sys

doc_path = pathlib.Path(sys.argv[1])
fail_on_mismatch = sys.argv[2] == "1"
text = doc_path.read_text(encoding="utf-8")

marker = "## Declaration vs Implementation Matrix (T-022 Deliverable)"
if marker not in text:
    print("[doc-sync] matrix marker missing: Declaration vs Implementation Matrix", file=sys.stderr)
    sys.exit(1)

matrix_text = text[text.index(marker):]
row_re = re.compile(r"^\|\s*([A-Z]{3,4}-\d{3})\s*\|")
rows: list[tuple[str, str, str]] = []
for line in matrix_text.splitlines():
    if not row_re.match(line):
        continue
    cols = [col.strip() for col in line.strip().strip("|").split("|")]
    if len(cols) < 3:
        print(f"[doc-sync] malformed matrix row: {line}", file=sys.stderr)
        sys.exit(1)
    contract_id, verdict, evidence = cols[0], cols[1], cols[2]
    rows.append((contract_id, verdict, evidence))

if not rows:
    print("[doc-sync] no contract rows found in matrix", file=sys.stderr)
    sys.exit(1)

seen = set()
dups = []
for contract_id, _, _ in rows:
    if contract_id in seen:
        dups.append(contract_id)
    seen.add(contract_id)
if dups:
    print(f"[doc-sync] duplicate contract ids: {', '.join(sorted(set(dups)))}", file=sys.stderr)
    sys.exit(1)

allowed_verdicts = {"match", "mismatch", "uncertain"}
invalid_verdicts = sorted({v for _, v, _ in rows if v not in allowed_verdicts})
if invalid_verdicts:
    print(f"[doc-sync] invalid verdict values: {', '.join(invalid_verdicts)}", file=sys.stderr)
    sys.exit(1)

def linked(evidence: str) -> bool:
    normalized = evidence.strip()
    return normalized not in {"", "-", "`-`", "N/A", "n/a", "none", "None"}

total = len(rows)
linked_rows = sum(1 for _, _, evidence in rows if linked(evidence))
coverage = linked_rows / total
missing_ids = [contract_id for contract_id, _, evidence in rows if not linked(evidence)]
verdict_counts = collections.Counter(verdict for _, verdict, _ in rows)

summary_errors: list[str] = []
summary_total = re.search(r"- Total declarations evaluated:\s*`(\d+)`", text)
summary_match = re.search(r"- `match`:\s*`(\d+)`", text)
summary_mismatch = re.search(r"- `mismatch`:\s*`(\d+)`", text)
summary_uncertain = re.search(r"- `uncertain`:\s*`(\d+)`", text)

if summary_total and int(summary_total.group(1)) != total:
    summary_errors.append(
        f"summary total mismatch (summary={summary_total.group(1)} computed={total})"
    )
if summary_match and int(summary_match.group(1)) != verdict_counts["match"]:
    summary_errors.append(
        f"summary match mismatch (summary={summary_match.group(1)} computed={verdict_counts['match']})"
    )
if summary_mismatch and int(summary_mismatch.group(1)) != verdict_counts["mismatch"]:
    summary_errors.append(
        f"summary mismatch mismatch (summary={summary_mismatch.group(1)} computed={verdict_counts['mismatch']})"
    )
if summary_uncertain and int(summary_uncertain.group(1)) != verdict_counts["uncertain"]:
    summary_errors.append(
        f"summary uncertain mismatch (summary={summary_uncertain.group(1)} computed={verdict_counts['uncertain']})"
    )

print(
    "[doc-sync] coverage:"
    f" total={total} linked={linked_rows} pct={coverage * 100:.2f}%"
)
print(
    "[doc-sync] verdicts:"
    f" match={verdict_counts['match']}"
    f" mismatch={verdict_counts['mismatch']}"
    f" uncertain={verdict_counts['uncertain']}"
)

if fail_on_mismatch and verdict_counts["mismatch"] > 0:
    print(
        f"[doc-sync] mismatch verdicts detected while strict mode is enabled: {verdict_counts['mismatch']}",
        file=sys.stderr,
    )
    sys.exit(1)

if summary_errors:
    for err in summary_errors:
        print(f"[doc-sync] {err}", file=sys.stderr)
    sys.exit(1)

if missing_ids:
    print(
        "[doc-sync] missing implementation evidence for: "
        + ", ".join(missing_ids),
        file=sys.stderr,
    )
    sys.exit(1)

print("[doc-sync] OK")
PY
)"
rc=$?
set -e

if [[ "$rc" -ne 0 ]]; then
  echo "$output" >&2
  if [[ "$MODE" == "soft" ]]; then
    echo "[doc-sync] warning only (soft mode)" >&2
    exit 0
  fi
  exit "$rc"
fi

echo "$output"
exit 0
