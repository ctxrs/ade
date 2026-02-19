#!/usr/bin/env bash
set -euo pipefail

REMOTE_HOST="${CTX_UPDATER_E2E_REMOTE_HOST:-${CTX_AUTOMATION_REMOTE_HOST:-}}"
REMOTE_USER="${CTX_UPDATER_E2E_REMOTE_USER:-${CTX_AUTOMATION_REMOTE_USER:-root}}"
REMOTE_CTX_BIN="${CTX_UPDATER_E2E_REMOTE_CTX_BIN:-${CTX_AUTOMATION_REMOTE_CTX_BIN:-/tmp/ctx-e2e-bin/ctx}}"
REMOTE_PORT="${CTX_AUTOMATION_REMOTE_PORT:-44099}"
REMOTE_DATA_DIR="${CTX_AUTOMATION_REMOTE_DATA_DIR:-/tmp/ctx-updater-e2e/daemon}"
SSH_KEY_PATH="${CTX_UPDATER_E2E_SSH_KEY_PATH:-}"
DOWNLOAD_BASE_URL="${CTX_UPDATER_E2E_DOWNLOAD_BASE_URL:-${SUPABASE_FUNCTIONS_URL:-https://api.ctx.rs/functions/v1}}"
RELEASE_CHANNEL="${RELEASE_CHANNEL:-stable}"
EXPECT_VERSION_CHANGE="${CTX_UPDATER_E2E_EXPECT_VERSION_CHANGE:-0}"

if [[ -z "$REMOTE_HOST" ]]; then
  echo "error: CTX_UPDATER_E2E_REMOTE_HOST/CTX_AUTOMATION_REMOTE_HOST is required" >&2
  exit 2
fi
if [[ -z "$SSH_KEY_PATH" ]]; then
  echo "error: CTX_UPDATER_E2E_SSH_KEY_PATH is required" >&2
  exit 2
fi

ssh_exec() {
  local remote_cmd="$1"
  ssh -i "$SSH_KEY_PATH" \
    -F /dev/null \
    -o BatchMode=yes \
    -o StrictHostKeyChecking=no \
    -o UserKnownHostsFile=/dev/null \
    -o ConnectTimeout=12 \
    "${REMOTE_USER}@${REMOTE_HOST}" \
    "$remote_cmd"
}

remote_sh() {
  local script="$1"
  ssh_exec "bash -lc $(printf '%q' "$script")"
}

stop_remote_daemon() {
  local script
  script="set -euo pipefail
mkdir -p $(printf '%q' "$REMOTE_DATA_DIR")
pid_file=$(printf '%q' "$REMOTE_DATA_DIR")/daemon.pid
if [[ -f \"$pid_file\" ]]; then
  pid=$(cat \"$pid_file\" || true)
  if [[ -n \"${pid:-}\" ]] && kill -0 \"$pid\" >/dev/null 2>&1; then
    kill \"$pid\" || true
    for _ in {1..20}; do
      if ! kill -0 \"$pid\" >/dev/null 2>&1; then
        break
      fi
      sleep 0.25
    done
    kill -9 \"$pid\" >/dev/null 2>&1 || true
  fi
  rm -f \"$pid_file\"
fi"
  remote_sh "$script" >/dev/null 2>&1 || true
}

start_remote_daemon() {
  local script
  script="set -euo pipefail
mkdir -p $(printf '%q' "$REMOTE_DATA_DIR")
if [[ ! -x $(printf '%q' "$REMOTE_CTX_BIN") ]]; then
  echo 'remote ctx binary is not executable' >&2
  exit 1
fi
log_file=$(printf '%q' "$REMOTE_DATA_DIR")/daemon.log
pid_file=$(printf '%q' "$REMOTE_DATA_DIR")/daemon.pid
nohup $(printf '%q' "$REMOTE_CTX_BIN") serve --bind 127.0.0.1:$(printf '%q' "$REMOTE_PORT") --data-dir $(printf '%q' "$REMOTE_DATA_DIR") >\"$log_file\" 2>&1 < /dev/null &
echo $! > \"$pid_file\""
  remote_sh "$script" >/dev/null
}

wait_remote_health() {
  local timeout_sec="${1:-45}"
  local script
  script="set -euo pipefail
for _ in $(seq 1 $(printf '%q' "$timeout_sec")); do
  if curl -fsS http://127.0.0.1:$(printf '%q' "$REMOTE_PORT")/api/health >/dev/null 2>&1; then
    exit 0
  fi
  sleep 1
done
echo 'remote daemon health check timed out' >&2
exit 1"
  remote_sh "$script" >/dev/null
}

remote_version() {
  remote_sh "set -euo pipefail; $(printf '%q' "$REMOTE_CTX_BIN") --version" | tr -d '\r' | tr -d '\n'
}

perform_remote_update() {
  local script
  script="set -euo pipefail
$(printf '%q' "$REMOTE_CTX_BIN") self-update --yes --channel $(printf '%q' "$RELEASE_CHANNEL") --base-url $(printf '%q' "$DOWNLOAD_BASE_URL")"
  remote_sh "$script"
}

cleanup() {
  stop_remote_daemon
}
trap cleanup EXIT INT TERM

before_version="$(remote_version)"
if [[ -z "$before_version" ]]; then
  echo "error: could not determine remote version" >&2
  exit 1
fi

stop_remote_daemon
start_remote_daemon
wait_remote_health 60
perform_remote_update >/tmp/ctx-updater-remote-update.log 2>&1 || {
  cat /tmp/ctx-updater-remote-update.log >&2 || true
  echo "error: remote self-update failed" >&2
  exit 1
}

stop_remote_daemon
start_remote_daemon
wait_remote_health 60

after_version="$(remote_version)"

if [[ "$EXPECT_VERSION_CHANGE" == "1" || "$EXPECT_VERSION_CHANGE" == "true" || "$EXPECT_VERSION_CHANGE" == "yes" ]]; then
  if [[ "$before_version" == "$after_version" ]]; then
    echo "error: expected version change but stayed at '$before_version'" >&2
    exit 1
  fi
fi

cat <<EOF_JSON
{"host":"$REMOTE_HOST","channel":"$RELEASE_CHANNEL","before_version":"$before_version","after_version":"$after_version","port":$REMOTE_PORT}
EOF_JSON
