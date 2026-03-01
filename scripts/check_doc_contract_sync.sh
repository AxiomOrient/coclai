#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DOC_MAP="${COCLAI_DOC_CONTRACT_MAP:-$ROOT/Docs/analysis/CONTRACT-MATRIX.md}"
MODE="${COCLAI_DOC_SYNC_MODE:-hard}" # hard|soft|off
FAIL_ON_MISMATCH="${COCLAI_DOC_SYNC_FAIL_ON_MISMATCH:-0}" # 1 => fail when mismatch verdict exists
VALIDATE_LINE_RANGES="${COCLAI_DOC_SYNC_VALIDATE_LINE_RANGES:-0}" # 1 => enforce file line upper bounds

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

case "$VALIDATE_LINE_RANGES" in
  0|1) ;;
  *)
    echo "[doc-sync] invalid COCLAI_DOC_SYNC_VALIDATE_LINE_RANGES=$VALIDATE_LINE_RANGES (allowed: 0|1)" >&2
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
tmp_out="$(mktemp)"
python3 - "$DOC_MAP" "$FAIL_ON_MISMATCH" "$ROOT" "$VALIDATE_LINE_RANGES" >"$tmp_out" <<'PY'
from __future__ import annotations

import collections
import pathlib
import re
import sys

doc_path = pathlib.Path(sys.argv[1])
fail_on_mismatch = sys.argv[2] == "1"
repo_root = pathlib.Path(sys.argv[3]).resolve()
validate_line_ranges = sys.argv[4] == "1"
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


def parse_line_reference(token: str) -> tuple[str, int, int | None] | None:
    match = re.match(r"^(.+?):(\d+)(?:-(\d+))?$", token)
    if not match:
        return None
    path_part = match.group(1).strip()
    start = int(match.group(2))
    end = int(match.group(3)) if match.group(3) else None
    return path_part, start, end


def candidate_refs(evidence: str) -> list[str]:
    refs = [segment.strip() for segment in re.findall(r"`([^`]+)`", evidence)]
    if refs:
        return [ref for ref in refs if ref]
    fallback = [segment.strip() for segment in evidence.split(",")]
    return [segment for segment in fallback if segment]


def path_token(candidate: str) -> str:
    token = candidate.strip().split()[0]
    return token.strip(",")


def looks_like_path(token: str) -> bool:
    if token in {"-", "N/A", "n/a", "none", "None"}:
        return False
    if "://" in token:
        return False
    if "/" in token:
        return True
    if token.endswith((".md", ".rs", ".toml", ".yml", ".yaml", ".sh", ".json")):
        return True
    return False


def resolve_under_repo(path_text: str) -> pathlib.Path:
    candidate = (repo_root / path_text).resolve()
    try:
        candidate.relative_to(repo_root)
    except ValueError:
        raise ValueError(f"path escapes repository root: {path_text}")
    return candidate


def validate_evidence_reference(ref: str) -> str | None:
    token = path_token(ref)
    parsed_line = parse_line_reference(token)
    path_text = token
    start: int | None = None
    end: int | None = None
    if parsed_line is not None:
        path_text, start, end = parsed_line

    if any(ch in path_text for ch in "*?[]"):
        if parsed_line is not None:
            return f"line reference cannot use glob path: {ref}"
        matches = list(repo_root.glob(path_text))
        if not matches:
            return f"glob path has no matches: {path_text}"
        return None

    try:
        candidate_path = resolve_under_repo(path_text)
    except ValueError as err:
        return str(err)

    if not candidate_path.exists():
        return f"missing path: {path_text}"

    if start is None:
        return None
    if not candidate_path.is_file():
        return f"line reference must target a file: {path_text}:{start}"

    if start < 1:
        return f"invalid line start (<1): {path_text}:{start}"
    if end is not None:
        if end < start:
            return f"invalid line range: {path_text}:{start}-{end}"
    if not validate_line_ranges:
        return None

    line_count = 0
    with candidate_path.open("r", encoding="utf-8", errors="replace") as handle:
        for line_count, _ in enumerate(handle, start=1):
            pass
    if line_count == 0:
        line_count = 0

    if start > line_count:
        return f"line start out of range: {path_text}:{start} (max={line_count})"
    if end is not None and end > line_count:
        return f"line end out of range: {path_text}:{start}-{end} (max={line_count})"
    return None

def linked(evidence: str) -> bool:
    normalized = evidence.strip()
    return normalized not in {"", "-", "`-`", "N/A", "n/a", "none", "None"}

total = len(rows)
linked_rows = sum(1 for _, _, evidence in rows if linked(evidence))
coverage = linked_rows / total
missing_ids = [contract_id for contract_id, _, evidence in rows if not linked(evidence)]
verdict_counts = collections.Counter(verdict for _, verdict, _ in rows)
evidence_errors: list[str] = []
checked_ref_count = 0

for contract_id, _, evidence in rows:
    for ref in candidate_refs(evidence):
        token = path_token(ref)
        if not looks_like_path(token):
            continue
        checked_ref_count += 1
        err = validate_evidence_reference(ref)
        if err is not None:
            evidence_errors.append(f"{contract_id}: {err}")

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
print(
    "[doc-sync] evidence refs:"
    f" checked={checked_ref_count} invalid={len(evidence_errors)}"
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

if evidence_errors:
    for err in evidence_errors:
        print(f"[doc-sync] invalid evidence reference: {err}", file=sys.stderr)
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
rc=$?
output="$(cat "$tmp_out")"
rm -f "$tmp_out"
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
