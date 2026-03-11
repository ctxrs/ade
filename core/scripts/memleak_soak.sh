#!/usr/bin/env bash
set -euo pipefail

PORT="${CTX_MEMLEAK_PORT:-4398}"
DURATION_SECS="${CTX_MEMLEAK_DURATION_SECS:-1800}"
SLOPE_MB_PER_MIN="${CTX_MEMLEAK_SLOPE_MB_PER_MIN:-5}"
CHURN_INTERVAL_SECS="${CTX_MEMLEAK_CHURN_INTERVAL_SECS:-10}"

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TIMESTAMP="$(date +%s)"
DATA_DIR="${CTX_MEMLEAK_DATA_DIR:-/tmp/ctx-memleak-soak-data-${TIMESTAMP}}"
REPO_DIR="${CTX_MEMLEAK_REPO_DIR:-/tmp/ctx-memleak-soak-repo-${TIMESTAMP}}"
LOG_PATH="${DATA_DIR}/logs/memleak-debug.jsonl"
DAEMON_LOG="${DATA_DIR}/daemon.out"

DAEMON_PID=""
WS_PID=""
CHURN_PID=""

cleanup() {
  if [[ -n "${CHURN_PID}" ]] && kill -0 "${CHURN_PID}" 2>/dev/null; then
    kill "${CHURN_PID}" 2>/dev/null || true
  fi
  if [[ -n "${WS_PID}" ]] && kill -0 "${WS_PID}" 2>/dev/null; then
    kill "${WS_PID}" 2>/dev/null || true
  fi
  if [[ -n "${DAEMON_PID}" ]] && kill -0 "${DAEMON_PID}" 2>/dev/null; then
    kill "${DAEMON_PID}" 2>/dev/null || true
  fi
}

trap cleanup EXIT

mkdir -p "${DATA_DIR}"
mkdir -p "${REPO_DIR}"

git -C "${REPO_DIR}" init -q
git -C "${REPO_DIR}" config user.email "memleak@example.com"
git -C "${REPO_DIR}" config user.name "ctx memleak soak"
echo "seed" > "${REPO_DIR}/seed.txt"
git -C "${REPO_DIR}" add seed.txt
git -C "${REPO_DIR}" commit -m "seed" -q

(
  cd "${ROOT_DIR}"
  CTX_MEMLEAK_DEBUG=1 \
  CTX_MEMLEAK_DEBUG_INTERVAL_MS=5000 \
  cargo run -p ctx-http -- serve --bind "127.0.0.1:${PORT}" --data-dir "${DATA_DIR}" \
    >"${DAEMON_LOG}" 2>&1 &
  DAEMON_PID=$!
  echo "${DAEMON_PID}" > "${DATA_DIR}/daemon.pid"
)
DAEMON_PID="$(cat "${DATA_DIR}/daemon.pid")"

echo "waiting for daemon on :${PORT}..."
READY=0
for _ in $(seq 1 60); do
  if curl -fsS "http://127.0.0.1:${PORT}/api/health" >/dev/null; then
    READY=1
    break
  fi
  sleep 1
done

if [[ "${READY}" -ne 1 ]]; then
  echo "daemon did not become ready; see ${DAEMON_LOG}"
  exit 1
fi

read -r WORKSPACE_ID TASK_ID SESSION_ID WORKTREE_ID <<EOF
$(PORT="${PORT}" REPO_DIR="${REPO_DIR}" python3 - <<'PY'
import json
import os
import urllib.request

port = os.environ["PORT"]
repo_dir = os.environ["REPO_DIR"]
base = f"http://127.0.0.1:{port}"

def post(url, payload):
    body = json.dumps(payload).encode("utf-8")
    req = urllib.request.Request(url, data=body, headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(req) as resp:
        return json.loads(resp.read().decode("utf-8"))

ws = post(f"{base}/api/workspaces", {"root_path": repo_dir, "name": "memleak-soak"})
task = post(f"{base}/api/workspaces/{ws['id']}/tasks", {"title": "memleak-soak"})
session = post(
    f"{base}/api/tasks/{task['id']}/sessions",
    {"provider_id": "fake", "model_id": "fake-model", "execution_environment": "host"},
)

print(ws["id"], task["id"], session["id"], session["worktree_id"])
PY
)
EOF

export PORT WORKSPACE_ID SESSION_ID DURATION_SECS
export LOG_PATH SLOPE_MB_PER_MIN

python3 - <<'PY' &
import base64
import json
import os
import socket
import time

host = "127.0.0.1"
port = int(os.environ["PORT"])
workspace_id = os.environ["WORKSPACE_ID"]
session_id = os.environ["SESSION_ID"]
duration = int(os.environ["DURATION_SECS"])

path = f"/api/workspaces/{workspace_id}/stream"
key = base64.b64encode(os.urandom(16)).decode("ascii")
req = (
    f"GET {path} HTTP/1.1\r\n"
    f"Host: {host}:{port}\r\n"
    "Upgrade: websocket\r\n"
    "Connection: Upgrade\r\n"
    f"Sec-WebSocket-Key: {key}\r\n"
    "Sec-WebSocket-Version: 13\r\n\r\n"
)

sock = socket.create_connection((host, port))
sock.sendall(req.encode("ascii"))
resp = sock.recv(4096)
if b"101" not in resp:
    raise SystemExit("websocket handshake failed")

def send_text(message: str) -> None:
    payload = message.encode("utf-8")
    mask = os.urandom(4)
    length = len(payload)
    header = bytearray()
    header.append(0x81)
    if length < 126:
        header.append(0x80 | length)
    elif length < 65536:
        header.append(0x80 | 126)
        header.extend(length.to_bytes(2, "big"))
    else:
        header.append(0x80 | 127)
        header.extend(length.to_bytes(8, "big"))
    masked = bytes(b ^ mask[i % 4] for i, b in enumerate(payload))
    sock.sendall(header + mask + masked)

subscribe = json.dumps({"type": "subscribe", "sessions": [{"session_id": session_id}]})
send_text(subscribe)

sock.settimeout(1.0)
end = time.time() + duration
while time.time() < end:
    try:
        sock.recv(4096)
    except socket.timeout:
        pass
PY
WS_PID=$!

if [[ "${CHURN_INTERVAL_SECS}" -gt 0 ]]; then
  while true; do
    printf "%s\n" "$(date -Iseconds)" > "${REPO_DIR}/churn.txt"
    sleep "${CHURN_INTERVAL_SECS}"
  done &
  CHURN_PID=$!
fi

echo "running soak for ${DURATION_SECS}s on :${PORT} (log: ${LOG_PATH})"
sleep "${DURATION_SECS}"

python3 - <<'PY'
import json
import os
from datetime import datetime

log_path = os.environ["LOG_PATH"]
threshold_mb_min = float(os.environ["SLOPE_MB_PER_MIN"])

with open(log_path, "r", encoding="utf-8") as f:
    lines = [line.strip() for line in f if line.strip()]

rows = []
for line in lines:
    rec = json.loads(line)
    ts = datetime.fromisoformat(rec["occurred_at"].replace("Z", "+00:00"))
    rows.append((ts, rec["rss_bytes"]))

if len(rows) < 2:
    raise SystemExit("not enough memleak samples")

rows.sort()
start_ts, start_rss = rows[0]
end_ts, end_rss = rows[-1]
secs = (end_ts - start_ts).total_seconds()
if secs <= 0:
    raise SystemExit("invalid sample window")

slope_bps = (end_rss - start_rss) / secs
threshold_bps = threshold_mb_min * 1024 * 1024 / 60.0
print(f"samples={len(rows)} window={secs:.1f}s slope={slope_bps:.2f} B/s")

if slope_bps > threshold_bps:
    raise SystemExit(
        f"slope {slope_bps:.2f} B/s exceeded threshold {threshold_bps:.2f} B/s"
    )
PY

echo "memleak soak passed"
