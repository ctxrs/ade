#!/usr/bin/env bash
set -euo pipefail

APP_PATH="${CTX_DESKTOP_APP_PATH:-/Users/example-user/code/ctx-monorepo/core/apps/desktop/src-tauri/target/release/bundle/macos/ctx.app}"
WORKSPACE_PATH="${CTX_DESKTOP_WORKSPACE_PATH:-/Users/example-user/code/ctx-monorepo}"
CTX_BIN="${CTX_DESKTOP_CTX_BIN:-/Users/example-user/code/ctx-monorepo/core/apps/desktop/src-tauri/bin/ctx-daemon}"
DATA_DIR="${CTX_DESKTOP_DAEMON_DATA_DIR:-/tmp/ctx-desktop-automation-$${RANDOM}}"

if [ ! -x "$CTX_BIN" ]; then
  echo "ctx binary not found or not executable: $CTX_BIN" >&2
  exit 1
fi
if [ ! -d "$APP_PATH" ]; then
  echo "ctx.app not found: $APP_PATH" >&2
  exit 1
fi
if [ ! -d "$WORKSPACE_PATH" ]; then
  echo "workspace path not found: $WORKSPACE_PATH" >&2
  exit 1
fi

PORT=$(python3 - <<'PY'
import socket
s=socket.socket()
s.bind(('127.0.0.1',0))
print(s.getsockname()[1])
s.close()
PY
)

mkdir -p "$DATA_DIR"
LOG="$DATA_DIR/daemon.log"

"$CTX_BIN" serve --bind "127.0.0.1:${PORT}" --data-dir "$DATA_DIR" >"$LOG" 2>&1 &
DAEMON_PID=$!
trap 'kill "$DAEMON_PID" >/dev/null 2>&1 || true' EXIT

for i in $(seq 1 50); do
  if curl -fsS "http://127.0.0.1:${PORT}/api/health" >/dev/null 2>&1; then
    break
  fi
  sleep 0.2
  if ! kill -0 "$DAEMON_PID" >/dev/null 2>&1; then
    echo "daemon exited early; log follows:" >&2
    tail -n 200 "$LOG" >&2 || true
    exit 1
  fi
done

AUTH="$DATA_DIR/daemon_auth.json"
if [ ! -f "$AUTH" ]; then
  echo "daemon_auth.json not found at $AUTH" >&2
  tail -n 200 "$LOG" >&2 || true
  exit 1
fi

TOKEN=$(python3 - <<PY
import json
with open("$AUTH","r") as f:
  print(json.load(f)["token"])
PY
)

export CTX_DESKTOP_DAEMON_URL="http://127.0.0.1:${PORT}"
export CTX_DESKTOP_DAEMON_TOKEN="$TOKEN"
export CTX_DESKTOP_START_WORKSPACE_PATHS="$WORKSPACE_PATH"

open "$APP_PATH"

echo "Opened ctx app with external daemon."
echo "Daemon URL: $CTX_DESKTOP_DAEMON_URL"
echo "Data dir: $DATA_DIR"
echo "Workspace: $WORKSPACE_PATH"

wait "$DAEMON_PID"
