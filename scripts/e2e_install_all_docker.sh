#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
BIN_PATH="$ROOT_DIR/core/target/debug/ctx"

if [[ ! -x "$BIN_PATH" ]]; then
  echo "[e2e] building ctx daemon (debug)"
  (cd "$ROOT_DIR/core" && cargo build -p ctx-http --bin ctx)
fi

if ! command -v docker >/dev/null 2>&1; then
  echo "[e2e] docker not found; install Docker first" >&2
  exit 1
fi

docker run --rm \
  -v "$BIN_PATH:/usr/local/bin/ctx:ro" \
  ubuntu:24.04 \
  bash -lc '
set -euo pipefail

export DEBIAN_FRONTEND=noninteractive
apt-get update -y >/dev/null
apt-get install -y curl jq git ca-certificates >/dev/null

DATA_DIR=/tmp/ctx-data
WORKSPACE=/tmp/ctx-workspace

mkdir -p "$WORKSPACE"
cd "$WORKSPACE"
git init -q
git config user.email "e2e@example.com"
git config user.name "E2E"
echo "ctx" > README.md
git add README.md
git commit -q -m "init"

ctx serve --bind 127.0.0.1:4399 --data-dir "$DATA_DIR" &
DAEMON_PID=$!
trap "kill $DAEMON_PID" EXIT

for _ in $(seq 1 60); do
  if curl -sf http://127.0.0.1:4399/api/health >/dev/null; then
    break
  fi
  sleep 1
done

WS_ID=$(curl -s -X POST http://127.0.0.1:4399/api/workspaces \
  -H "content-type: application/json" \
  -d "{\"root_path\":\"$WORKSPACE\"}" | jq -r ".id")
if [[ -z "$WS_ID" || "$WS_ID" == "null" ]]; then
  echo "[e2e] failed to create workspace" >&2
  exit 1
fi

TASK_ID=$(curl -s -X POST "http://127.0.0.1:4399/api/workspaces/$WS_ID/tasks" \
  -H "content-type: application/json" \
  -d "{\"title\":\"e2e install all\",\"create_default_track\":true}" | jq -r ".id")
if [[ -z "$TASK_ID" || "$TASK_ID" == "null" ]]; then
  echo "[e2e] failed to create task" >&2
  exit 1
fi

TRACK_ID=$(curl -s "http://127.0.0.1:4399/api/tasks/$TASK_ID/tracks" | jq -r ".[0].id")
if [[ -z "$TRACK_ID" || "$TRACK_ID" == "null" ]]; then
  echo "[e2e] failed to fetch track" >&2
  exit 1
fi

INSTALLS=$(curl -s -X POST http://127.0.0.1:4399/api/providers/install_all)
if [[ "$INSTALLS" != "[]" ]]; then
  echo "$INSTALLS" | jq "."
  echo "$INSTALLS" | jq -r ".[].install_id" | while read -r id; do
    for _ in $(seq 1 240); do
      state=$(curl -s "http://127.0.0.1:4399/api/providers/install/$id" | jq -r ".state")
      if [[ "$state" == "succeeded" ]]; then
        break
      fi
      if [[ "$state" == "failed" ]]; then
        echo "[e2e] install failed for $id" >&2
        curl -s "http://127.0.0.1:4399/api/providers/install/$id" | jq "." >&2
        curl -s "http://127.0.0.1:4399/api/providers/install/$id/events" | jq "." >&2
        exit 1
      fi
      sleep 2
    done
  done
fi

PROVIDERS=$(curl -s http://127.0.0.1:4399/api/providers | jq -r ".[].provider_id")
for pid in $PROVIDERS; do
  curl -s -X POST "http://127.0.0.1:4399/api/tracks/$TRACK_ID/sessions" \
    -H "content-type: application/json" \
    -d "{\"provider_id\":\"$pid\",\"model_id\":\"default\"}" >/dev/null
  echo "[e2e] created session for $pid"
done

echo "[e2e] complete"
'
