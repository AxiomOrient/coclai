#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BASELINE_PATH="${CODEX_PERF_BASELINE:-$ROOT_DIR/SCHEMAS/golden/perf/micro_bench_baseline.json}"
OUT_PATH="${CODEX_PERF_OUT:-$ROOT_DIR/target/perf/micro_latest.json}"
ITERATIONS="${CODEX_PERF_ITERATIONS:-150000}"
WARMUP="${CODEX_PERF_WARMUP:-20000}"
MAX_REGRESSION="${CODEX_PERF_MAX_REGRESSION:-0.15}"
MAX_HOOK_LINEARITY="${CODEX_PERF_MAX_HOOK_LINEARITY:-0.50}"
WRITE_BASELINE="${CODEX_PERF_WRITE_BASELINE:-0}"
RETRIES="${CODEX_PERF_RETRIES:-2}"

cd "$ROOT_DIR"

if [[ "$WRITE_BASELINE" == "1" ]]; then
  mkdir -p "$(dirname "$BASELINE_PATH")"
  cargo run -p coclai_runtime --bin perf_micro_bench --release -- \
    --out "$BASELINE_PATH" \
    --iterations "$ITERATIONS" \
    --warmup "$WARMUP" \
    --max-hook-linearity "$MAX_HOOK_LINEARITY"
  echo "wrote micro-bench baseline: $BASELINE_PATH"
  exit 0
fi

ARGS=(
  --out "$OUT_PATH"
  --iterations "$ITERATIONS"
  --warmup "$WARMUP"
)

if [[ -f "$BASELINE_PATH" ]]; then
  ARGS+=(
    --baseline "$BASELINE_PATH"
    --max-regression "$MAX_REGRESSION"
    --max-hook-linearity "$MAX_HOOK_LINEARITY"
  )
else
  ARGS+=(
    --max-hook-linearity "$MAX_HOOK_LINEARITY"
  )
fi

if [[ ! -f "$BASELINE_PATH" ]]; then
  cargo run -p coclai_runtime --bin perf_micro_bench --release -- "${ARGS[@]}"
  echo "micro-bench report: $OUT_PATH"
  exit 0
fi

attempt=1
while true; do
  if cargo run -p coclai_runtime --bin perf_micro_bench --release -- "${ARGS[@]}"; then
    if (( attempt > 1 )); then
      echo "micro-bench regression check passed on retry attempt $attempt"
    fi
    break
  fi

  if (( attempt > RETRIES )); then
    echo "micro-bench regression check failed after $attempt attempt(s)" >&2
    exit 1
  fi

  echo "micro-bench regression detected on attempt $attempt; retrying..." >&2
  attempt=$((attempt + 1))
done

echo "micro-bench report: $OUT_PATH"
