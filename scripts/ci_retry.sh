#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -lt 1 ]; then
  echo "usage: $0 <command> [args...]" >&2
  exit 2
fi

attempts="${CI_RETRY_ATTEMPTS:-2}"
delay="${CI_RETRY_DELAY_SEC:-5}"
log_file="${CI_FLAKE_LOG:-ci-flake-stats.jsonl}"
job_name="${CI_JOB_NAME:-${GITHUB_JOB:-unknown}}"
fail_on_flake_raw="${CI_FAIL_ON_FLAKE:-0}"
fail_on_flake_norm="$(printf '%s' "$fail_on_flake_raw" | tr '[:upper:]' '[:lower:]')"

case "$fail_on_flake_norm" in
  1|true|yes|on) fail_on_flake=1 ;;
  *) fail_on_flake=0 ;;
esac

if ! [[ "$attempts" =~ ^[0-9]+$ ]] || [ "$attempts" -lt 1 ]; then
  echo "CI_RETRY_ATTEMPTS must be a positive integer" >&2
  exit 2
fi

command=("$@")
command_str="$*"
last_exit=0

json_escape() {
  python3 - <<'PY' "$1"
import json
import sys
print(json.dumps(sys.argv[1]))
PY
}

record_event() {
  local status_label="$1"
  local exit_code="$2"
  local attempts_used="$3"
  local ts
  ts=$(date -u +"%Y-%m-%dT%H:%M:%SZ")
  local esc_job
  local esc_cmd
  esc_job=$(json_escape "$job_name")
  esc_cmd=$(json_escape "$command_str")
  printf '{"ts":"%s","job":%s,"status":"%s","attempts":%d,"exit_code":%d,"command":%s}\n' \
    "$ts" "$esc_job" "$status_label" "$attempts_used" "$exit_code" "$esc_cmd" >> "$log_file"
}

attempt=1
while true; do
  if "${command[@]}"; then
    if [ "$attempt" -gt 1 ]; then
      if [ "$fail_on_flake" -eq 1 ]; then
        record_event "flake_failed" "$last_exit" "$attempt"
        echo "command succeeded after retry, but CI_FAIL_ON_FLAKE=1 is set; failing job" >&2
        exit 86
      fi
      record_event "flake" "$last_exit" "$attempt"
    fi
    exit 0
  fi

  last_exit=$?
  if [ "$attempt" -ge "$attempts" ]; then
    record_event "failed" "$last_exit" "$attempt"
    exit "$last_exit"
  fi

  attempt=$((attempt + 1))
  sleep "$delay"
done
