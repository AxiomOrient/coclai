#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT_DIR"

PKG="${CODEX_RUNTIME_PKG:-codex-runtime}"
INCLUDE_REAL_SERVER="${CODEX_RUNTIME_RELEASE_INCLUDE_REAL_SERVER:-0}"
REAL_SERVER_RETRIES="${CODEX_RUNTIME_RELEASE_REAL_SERVER_RETRIES:-3}"
REAL_SERVER_BACKOFF_SEC="${CODEX_RUNTIME_RELEASE_REAL_SERVER_BACKOFF_SEC:-3}"
REAL_SERVER_APPROVED="${CODEX_RUNTIME_REAL_SERVER_APPROVED:-0}"

case "$INCLUDE_REAL_SERVER" in
  0|1) ;;
  *)
    echo "[release] invalid CODEX_RUNTIME_RELEASE_INCLUDE_REAL_SERVER=$INCLUDE_REAL_SERVER (allowed: 0|1)" >&2
    exit 2
    ;;
esac

case "$REAL_SERVER_RETRIES" in
  ''|*[!0-9]*)
    echo "[release] invalid CODEX_RUNTIME_RELEASE_REAL_SERVER_RETRIES=$REAL_SERVER_RETRIES (allowed: integer >= 1)" >&2
    exit 2
    ;;
esac

if [ "$REAL_SERVER_RETRIES" -lt 1 ]; then
  echo "[release] invalid CODEX_RUNTIME_RELEASE_REAL_SERVER_RETRIES=$REAL_SERVER_RETRIES (allowed: integer >= 1)" >&2
  exit 2
fi

case "$REAL_SERVER_BACKOFF_SEC" in
  ''|*[!0-9]*)
    echo "[release] invalid CODEX_RUNTIME_RELEASE_REAL_SERVER_BACKOFF_SEC=$REAL_SERVER_BACKOFF_SEC (allowed: non-negative integer)" >&2
    exit 2
    ;;
esac

case "$REAL_SERVER_APPROVED" in
  0|1) ;;
  *)
    echo "[release] invalid CODEX_RUNTIME_REAL_SERVER_APPROVED=$REAL_SERVER_APPROVED (allowed: 0|1)" >&2
    exit 2
    ;;
esac

if ! command -v python3 >/dev/null 2>&1; then
  echo "[release] python3 is required for mock-process runtime tests (runtime/api/core fixtures)" >&2
  exit 2
fi

check_release_docs_sync() {
  python3 - <<'PY'
from pathlib import Path
import re
import sys


def fail(message: str) -> None:
    print(f"[release-docs] {message}", file=sys.stderr)
    raise SystemExit(1)


def read(path: str) -> str:
    return Path(path).read_text(encoding="utf-8")


def extract_version(text: str, section: str) -> str:
    pattern = rf'(?ms)^\[{re.escape(section)}\]\n(?:(?!^\[).*\n)*?^version = "([^"]+)"'
    match = re.search(pattern, text)
    if not match:
        fail(f"unable to read version from [{section}]")
    return match.group(1)


workspace_version = extract_version(read("Cargo.toml"), "workspace.package")
crate_version = extract_version(read("crates/codex-runtime/Cargo.toml"), "package")

if workspace_version != crate_version:
    fail(
        f"workspace version {workspace_version} does not match crate version {crate_version}"
    )

lock_text = read("Cargo.lock")
lock_match = re.search(
    r'(?ms)^\[\[package\]\]\nname = "codex-runtime"\nversion = "([^"]+)"',
    lock_text,
)
if not lock_match:
    fail("Cargo.lock is missing the codex-runtime package entry")
lock_version = lock_match.group(1)
if lock_version != workspace_version:
    fail(
        f"Cargo.lock version {lock_version} does not match workspace version {workspace_version}"
    )

readme = read("README.md")
if f'codex-runtime = "{workspace_version}"' not in readme:
    fail(f'README.md is missing dependency version "{workspace_version}"')

changelog = read("CHANGELOG.md")
changelog_match = re.search(r"^## \[([^\]]+)\]", changelog, re.MULTILINE)
if not changelog_match:
    fail("CHANGELOG.md is missing a release heading")
if changelog_match.group(1) != workspace_version:
    fail(
        f"CHANGELOG.md latest release {changelog_match.group(1)} does not match {workspace_version}"
    )

release_doc = Path("docs/releases") / f"{workspace_version}.md"
if not release_doc.exists():
    fail(f"missing release note {release_doc}")
release_title = re.search(r"^# codex-runtime ([^\n]+)", read(str(release_doc)), re.MULTILINE)
if not release_title:
    fail(f"{release_doc} is missing the release title")
if release_title.group(1) != workspace_version:
    fail(
        f"{release_doc} title version {release_title.group(1)} does not match {workspace_version}"
    )

spec = read("SPEC.md")
spec_status = re.search(r"^Status:\s*(.+)$", spec, re.MULTILINE)
if not spec_status:
    fail("SPEC.md is missing a status line")
minor_line = ".".join(workspace_version.split(".")[:2]) + ".x"
if f"`{workspace_version}`" not in spec_status.group(1) and f"`{minor_line}`" not in spec_status.group(1):
    fail(
        "SPEC.md status line must mention the current release version or current major.minor line"
    )

print(f"[release-docs] version sync ok: {workspace_version}")
PY
}

run_real_server_gate_with_retries() {
  local max_attempts="$1"
  local backoff_sec="$2"
  local test_filter="$3"
  local attempt=1
  while (( attempt <= max_attempts )); do
    echo "[release] gate: ${PKG} real-server test '${test_filter}' (attempt ${attempt}/${max_attempts})"
    if cargo test -p "$PKG" "$test_filter" -- --ignored --nocapture; then
      return 0
    fi
    if (( attempt == max_attempts )); then
      echo "[release] ${PKG} real-server test '${test_filter}' exhausted retries" >&2
      return 1
    fi
    echo "[release] ${PKG} real-server test '${test_filter}' failed; retrying in ${backoff_sec}s" >&2
    sleep "$backoff_sec"
    attempt=$((attempt + 1))
  done
}

echo "[release] gate: fmt"
cargo fmt --check

echo "[release] gate: clippy"
cargo clippy --workspace --all-targets -- -D warnings

echo "[release] gate: release docs"
check_release_docs_sync

echo "[release] gate: product hygiene"
./scripts/check_product_hygiene.sh

echo "[release] gate: security"
./scripts/check_security_gate.sh

echo "[release] gate: blocker regressions"
./scripts/check_blocker_regressions.sh

echo "[release] gate: tests"
cargo test --workspace

if [[ "$INCLUDE_REAL_SERVER" == "1" ]]; then
  if [[ "$REAL_SERVER_APPROVED" != "1" ]]; then
    echo "[release] refusing real-server gate: set CODEX_RUNTIME_REAL_SERVER_APPROVED=1 after explicit operator approval" >&2
    exit 2
  fi
  echo "[release] gate: real-server contract"
  run_real_server_gate_with_retries \
    "$REAL_SERVER_RETRIES" \
    "$REAL_SERVER_BACKOFF_SEC" \
    "ergonomic::tests::real_server::"
else
  echo "[release] gate: real-server contract (skipped; set CODEX_RUNTIME_RELEASE_INCLUDE_REAL_SERVER=1)"
fi

echo "[release] preflight passed"
