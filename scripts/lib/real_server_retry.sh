#!/usr/bin/env bash

# shellcheck shell=bash

run_real_server_gate_with_retries() {
  local max_attempts="$1"
  local backoff_sec="$2"
  local test_filter="$3"
  local log_path="${4:-}"
  local label="${5:-real-server gate}"
  local attempt=1

  while (( attempt <= max_attempts )); do
    echo "${label}: '${test_filter}' (attempt ${attempt}/${max_attempts})"
    if [[ -n "$log_path" ]]; then
      if cargo test -p coclai "$test_filter" -- --nocapture 2>&1 | tee "$log_path"; then
        return 0
      fi
    else
      if cargo test -p coclai "$test_filter" -- --nocapture; then
        return 0
      fi
    fi

    if (( attempt == max_attempts )); then
      echo "${label}: '${test_filter}' exhausted retries" >&2
      return 1
    fi

    echo "${label}: '${test_filter}' failed; retrying in ${backoff_sec}s" >&2
    sleep "$backoff_sec"
    attempt=$((attempt + 1))
  done
}
