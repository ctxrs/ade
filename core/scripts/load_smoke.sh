#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
core_root="$(cd "${script_dir}/.." && pwd)"
cd "${core_root}"

export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/.cache/cargo/ctx-monorepo/$(basename "$(git rev-parse --git-dir)")}"

if ! command -v python3 >/dev/null 2>&1; then
  echo "error: python3 is required for load_smoke.sh" >&2
  exit 1
fi
if ! command -v curl >/dev/null 2>&1; then
  echo "error: curl is required for load_smoke.sh" >&2
  exit 1
fi

pick_port() {
  python3 - <<'PY'
import socket
s=socket.socket()
s.bind(("127.0.0.1",0))
print(s.getsockname()[1])
s.close()
PY
}

PORT="${CTX_LOAD_SMOKE_PORT:-$(pick_port)}"
DURATION_MS="${CTX_LOAD_SMOKE_DURATION_MS:-15000}"
TASKS="${CTX_LOAD_SMOKE_TASKS:-4}"
SUBAGENTS_MIN="${CTX_LOAD_SMOKE_SUBAGENTS_MIN:-1}"
SUBAGENTS_MAX="${CTX_LOAD_SMOKE_SUBAGENTS_MAX:-2}"
MESSAGE_INTERVAL_MS="${CTX_LOAD_SMOKE_MESSAGE_INTERVAL_MS:-300}"
READY_TIMEOUT_SECS="${CTX_LOAD_SMOKE_READY_TIMEOUT_SECS:-300}"

TMP_ROOT="${CTX_LOAD_SMOKE_TMP_DIR:-$(mktemp -d /tmp/ctx-load-smoke.XXXXXX)}"
REPO_DIR="${TMP_ROOT}/repo"
DATA_DIR="${TMP_ROOT}/data"
OUT_DIR="${TMP_ROOT}/out"
LOG_PATH="${OUT_DIR}/daemon.log"
mkdir -p "${REPO_DIR}" "${DATA_DIR}" "${OUT_DIR}"

cleanup() {
  if [[ -n "${DAEMON_PID:-}" ]]; then
    kill "${DAEMON_PID}" >/dev/null 2>&1 || true
    wait "${DAEMON_PID}" >/dev/null 2>&1 || true
  fi
  if [[ -z "${CTX_LOAD_SMOKE_KEEP_TMP:-}" ]]; then
    rm -rf "${TMP_ROOT}"
  else
    echo "load smoke artifacts retained at ${TMP_ROOT}"
  fi
}
trap cleanup EXIT

# Hermetic tiny git workspace for the load run.
git -C "${REPO_DIR}" init >/dev/null 2>&1
git -C "${REPO_DIR}" config user.email test@example.com >/dev/null
git -C "${REPO_DIR}" config user.name Test >/dev/null
echo "load smoke" > "${REPO_DIR}/README.md"
git -C "${REPO_DIR}" add . >/dev/null
git -C "${REPO_DIR}" commit -m init >/dev/null

CTX_SHOW_FAKE_PROVIDER=1 cargo run -p ctx-http --bin ctx -- serve \
  --bind "127.0.0.1:${PORT}" --data-dir "${DATA_DIR}" \
  >"${LOG_PATH}" 2>&1 &
DAEMON_PID="$!"

ready_attempts=$((READY_TIMEOUT_SECS * 2))
for _ in $(seq 1 "${ready_attempts}"); do
  if curl -fsS "http://127.0.0.1:${PORT}/api/health" >/dev/null 2>&1; then
    break
  fi
  sleep 0.5
done

if ! curl -fsS "http://127.0.0.1:${PORT}/api/health" >/dev/null 2>&1; then
  echo "error: daemon failed to become healthy; tailing log" >&2
  tail -n 120 "${LOG_PATH}" >&2 || true
  exit 1
fi

AUTH_FILE="${DATA_DIR}/daemon_auth.json"
if [[ ! -f "${AUTH_FILE}" ]]; then
  echo "error: missing auth file at ${AUTH_FILE}" >&2
  exit 1
fi

AUTH_TOKEN="$(python3 - <<'PY' "${AUTH_FILE}"
import json,sys
with open(sys.argv[1], 'r', encoding='utf-8') as f:
    data=json.load(f)
print(data.get('token',''))
PY
)"

if [[ -z "${AUTH_TOKEN}" ]]; then
  echo "error: failed to read daemon auth token" >&2
  exit 1
fi

WORKSPACE_JSON="$(curl -fsS \
  -H "Authorization: Bearer ${AUTH_TOKEN}" \
  -H "content-type: application/json" \
  -d "{\"root_path\":\"${REPO_DIR}\",\"name\":\"load-smoke\"}" \
  "http://127.0.0.1:${PORT}/api/workspaces")"

WORKSPACE_ID="$(python3 - <<'PY' "${WORKSPACE_JSON}"
import json,sys
obj=json.loads(sys.argv[1])
print(obj.get('id',''))
PY
)"

if [[ -z "${WORKSPACE_ID}" ]]; then
  echo "error: failed to create workspace for load smoke" >&2
  exit 1
fi

run_out_dir="${OUT_DIR}/load"
mkdir -p "${run_out_dir}"

cargo run -p ctx-load-test -- \
  --scenario tools/load-test/examples/baseline.json \
  --base-url "http://127.0.0.1:${PORT}" \
  --auth-token "${AUTH_TOKEN}" \
  --workspace-id "${WORKSPACE_ID}" \
  --out-dir "${run_out_dir}" \
  --duration-ms "${DURATION_MS}" \
  --tasks "${TASKS}" \
  --subagents-min "${SUBAGENTS_MIN}" \
  --subagents-max "${SUBAGENTS_MAX}" \
  --message-interval-ms "${MESSAGE_INTERVAL_MS}" \
  --provider-id fake \
  --model-id fake-model

echo "load smoke completed; summary at ${run_out_dir}/summary.json"
