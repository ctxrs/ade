#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)

PORT=${CTX_PERF_PORT:-4404}
DATA_DIR=${CTX_PERF_DATA_DIR:-/tmp/ctx-perf-smoke}
DURATION_SECS=${CTX_PERF_DURATION_SECS:-30}
INTERVAL_SECS=${CTX_PERF_INTERVAL_SECS:-1}
TELEMETRY_INTERVAL_MS=${CTX_PERF_TELEMETRY_INTERVAL_MS:-5000}
LOG_PATH=${CTX_PERF_LOG_PATH:-/tmp/ctx-perf-smoke.log}
PIDSTAT_PATH=${CTX_PERF_PIDSTAT_PATH:-/tmp/ctx-perf-smoke-pidstat.txt}
MAX_CPU_PCT=${CTX_PERF_MAX_CPU_PCT:-50}
NO_BUILD=${CTX_PERF_NO_BUILD:-0}

CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$HOME/.cache/cargo/ctx-monorepo}
CTX_BIN="$CARGO_TARGET_DIR/debug/ctx"

if command -v lsof >/dev/null 2>&1; then
  if lsof -nP -iTCP:"$PORT" -sTCP:LISTEN >/dev/null 2>&1; then
    echo "error: port $PORT is already in use." >&2
    echo "hint: set CTX_PERF_PORT to a free port." >&2
    exit 1
  fi
elif command -v ss >/dev/null 2>&1; then
  if ss -ltn "sport = :$PORT" | tail -n +2 | grep -q .; then
    echo "error: port $PORT is already in use." >&2
    echo "hint: set CTX_PERF_PORT to a free port." >&2
    exit 1
  fi
fi

if [ "$NO_BUILD" != "1" ]; then
  (cd "$ROOT_DIR/core" && CARGO_TARGET_DIR="$CARGO_TARGET_DIR" cargo build -p ctx-http --bin ctx)
fi

if [ ! -x "$CTX_BIN" ]; then
  echo "error: ctx binary not found at $CTX_BIN" >&2
  echo "hint: run with CTX_PERF_NO_BUILD=0 or set CARGO_TARGET_DIR." >&2
  exit 1
fi

rm -rf "$DATA_DIR"
mkdir -p "$DATA_DIR"
: > "$LOG_PATH"

RUST_LOG=${RUST_LOG:-info} \
CTX_DAEMON_LOG_STDOUT=1 \
CTX_DAEMON_LOG_STDOUT_FILTER=${CTX_DAEMON_LOG_STDOUT_FILTER:-error} \
CTX_DAEMON_LOG_MAX_BYTES=0 \
CTX_RESOURCE_TELEMETRY_INTERVAL_MS="$TELEMETRY_INTERVAL_MS" \
CTX_RESOURCE_TELEMETRY_LOCAL_MAX_BYTES=10485760 \
CTX_ACP_HEAP_PROFILE=0 \
"$CTX_BIN" serve --bind "127.0.0.1:$PORT" --data-dir "$DATA_DIR" >> "$LOG_PATH" 2>&1 &
PID=$!

cleanup() {
  if kill -0 "$PID" >/dev/null 2>&1; then
    kill "$PID" >/dev/null 2>&1 || true
    wait "$PID" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

for _ in $(seq 1 40); do
  if command -v lsof >/dev/null 2>&1; then
    if lsof -nP -iTCP:"$PORT" -sTCP:LISTEN >/dev/null 2>&1; then
      break
    fi
  elif command -v ss >/dev/null 2>&1; then
    if ss -ltn "sport = :$PORT" | tail -n +2 | grep -q .; then
      break
    fi
  fi
  sleep 0.25
  if ! kill -0 "$PID" >/dev/null 2>&1; then
    echo "error: daemon exited early (pid $PID)." >&2
    tail -n 50 "$LOG_PATH" >&2 || true
    exit 1
  fi
  if [ "$SECONDS" -gt 10 ]; then
    echo "error: daemon did not bind within 10s." >&2
    tail -n 50 "$LOG_PATH" >&2 || true
    exit 1
  fi

done

pidstat -u -p "$PID" "$INTERVAL_SECS" "$DURATION_SECS" > "$PIDSTAT_PATH"

avg_line=$(awk '$1=="Average:" && $2 ~ /^[0-9]+$/ {print $0}' "$PIDSTAT_PATH" | tail -n 1)
if [ -z "$avg_line" ]; then
  echo "error: could not find Average line in pidstat output." >&2
  tail -n 20 "$PIDSTAT_PATH" >&2 || true
  exit 1
fi

avg_cpu=$(awk '$1=="Average:" && $2 ~ /^[0-9]+$/ {print $8}' "$PIDSTAT_PATH" | tail -n 1)
avg_usr=$(awk '$1=="Average:" && $2 ~ /^[0-9]+$/ {print $4}' "$PIDSTAT_PATH" | tail -n 1)
avg_sys=$(awk '$1=="Average:" && $2 ~ /^[0-9]+$/ {print $5}' "$PIDSTAT_PATH" | tail -n 1)

printf 'perf-smoke: avg_cpu=%.2f%% (usr=%s sys=%s) over %ss\n' "$avg_cpu" "$avg_usr" "$avg_sys" "$DURATION_SECS"

if [ "${MAX_CPU_PCT}" != "" ]; then
  exceed=$(awk -v cpu="$avg_cpu" -v max="$MAX_CPU_PCT" 'BEGIN {print (cpu > max) ? 1 : 0}')
  if [ "$exceed" = "1" ]; then
    echo "error: avg CPU ${avg_cpu}% exceeds threshold ${MAX_CPU_PCT}%" >&2
    exit 1
  fi
fi
