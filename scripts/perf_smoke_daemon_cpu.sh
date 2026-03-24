#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)

PORT=${CTX_PERF_PORT:-4404}
DATA_DIR=${CTX_PERF_DATA_DIR:-/tmp/ctx-perf-smoke}
DURATION_SECS=${CTX_PERF_DURATION_SECS:-30}
TELEMETRY_INTERVAL_MS=${CTX_PERF_TELEMETRY_INTERVAL_MS:-5000}
LOG_PATH=${CTX_PERF_LOG_PATH:-/tmp/ctx-perf-smoke.log}
MAX_CPU_PCT=${CTX_PERF_MAX_CPU_PCT:-50}
MAX_RSS_MB=${CTX_PERF_MAX_RSS_MB:-}
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
CTX_RESOURCE_UTILIZATION_DISABLED=0 \
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

bind_deadline=$((SECONDS + 10))
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
  if [ "$SECONDS" -ge "$bind_deadline" ]; then
    echo "error: daemon did not bind within 10s." >&2
    tail -n 50 "$LOG_PATH" >&2 || true
    exit 1
  fi

done

deadline=$((SECONDS + DURATION_SECS))
while [ "$SECONDS" -lt "$deadline" ]; do
  sleep 1
  if ! kill -0 "$PID" >/dev/null 2>&1; then
    echo "error: daemon exited during perf smoke (pid $PID)." >&2
    tail -n 50 "$LOG_PATH" >&2 || true
    exit 1
  fi
done

resource_summary=$(
  python3 - "$DATA_DIR" <<'PY'
import json
import pathlib
import sys

data_dir = pathlib.Path(sys.argv[1])
log_dir = data_dir / "logs"
files = sorted(log_dir.glob("resource-util-*.jsonl"))
if not files:
    raise SystemExit(
        f"error: no resource telemetry logs found under {log_dir}. "
        "Check CTX_RESOURCE_TELEMETRY_INTERVAL_MS and daemon startup logs."
    )

samples = []
for path in files:
    with path.open("r", encoding="utf-8") as handle:
        for raw_line in handle:
            line = raw_line.strip()
            if not line:
                continue
            try:
                row = json.loads(line)
            except json.JSONDecodeError:
                continue
            daemon = (row.get("processes") or {}).get("daemon")
            if not daemon:
                continue
            cpu_pct = daemon.get("cpu_pct")
            memory_bytes = daemon.get("memory_bytes")
            if cpu_pct is None or memory_bytes is None:
                continue
            samples.append((float(cpu_pct), int(memory_bytes)))

if len(samples) < 2:
    raise SystemExit(
        "error: need at least 2 daemon telemetry samples for perf smoke; "
        f"found {len(samples)}."
    )

cpu_values = [sample[0] for sample in samples]
memory_values = [sample[1] for sample in samples]

print(f"sample_count={len(samples)}")
print(f"avg_cpu_pct={sum(cpu_values) / len(cpu_values):.2f}")
print(f"peak_cpu_pct={max(cpu_values):.2f}")
print(f"avg_rss_mb={sum(memory_values) / len(memory_values) / (1024 * 1024):.2f}")
print(f"peak_rss_mb={max(memory_values) / (1024 * 1024):.2f}")
PY
)

sample_count=""
avg_cpu=""
peak_cpu=""
avg_rss=""
peak_rss=""
while IFS='=' read -r key value; do
  case "$key" in
    sample_count) sample_count="$value" ;;
    avg_cpu_pct) avg_cpu="$value" ;;
    peak_cpu_pct) peak_cpu="$value" ;;
    avg_rss_mb) avg_rss="$value" ;;
    peak_rss_mb) peak_rss="$value" ;;
  esac
done <<< "$resource_summary"

if [ -z "$sample_count" ] || [ -z "$avg_cpu" ] || [ -z "$peak_cpu" ] || [ -z "$avg_rss" ] || [ -z "$peak_rss" ]; then
  echo "error: failed to parse resource telemetry summary." >&2
  printf '%s\n' "$resource_summary" >&2
  exit 1
fi

printf \
  'perf-smoke: avg_cpu=%s%% peak_cpu=%s%% avg_rss=%sMiB peak_rss=%sMiB samples=%s over %ss\n' \
  "$avg_cpu" "$peak_cpu" "$avg_rss" "$peak_rss" "$sample_count" "$DURATION_SECS"

if [ "${MAX_CPU_PCT}" != "" ]; then
  exceed=$(awk -v cpu="$avg_cpu" -v max="$MAX_CPU_PCT" 'BEGIN {print (cpu > max) ? 1 : 0}')
  if [ "$exceed" = "1" ]; then
    echo "error: avg CPU ${avg_cpu}% exceeds threshold ${MAX_CPU_PCT}%" >&2
    exit 1
  fi
fi

if [ "${MAX_RSS_MB}" != "" ]; then
  exceed=$(awk -v rss="$peak_rss" -v max="$MAX_RSS_MB" 'BEGIN {print (rss > max) ? 1 : 0}')
  if [ "$exceed" = "1" ]; then
    echo "error: peak RSS ${peak_rss}MiB exceeds threshold ${MAX_RSS_MB}MiB" >&2
    exit 1
  fi
fi
