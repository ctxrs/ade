#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")"/../../../.. && pwd)"
FIXTURE_SCRIPT="${ROOT}/core/apps/desktop/scripts/remote_ssh_fixture.sh"

RUNTIME="auto"
KEEP_ALIVE=0
LOG_DIR=""
STATE_FILE="${CTX_AUTOMATION_REMOTE_FIXTURE_STATE_FILE:-}"
PASSTHROUGH_ARGS=()

usage() {
  cat <<'USAGE' >&2
usage:
  test_remote_bootstrap_fixture.sh [--runtime auto|docker|podman] [--log-dir PATH] [--keep-alive] [--state-file PATH] [-- ...wdio args]
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --runtime)
      RUNTIME="${2:-}"
      shift 2
      ;;
    --log-dir)
      LOG_DIR="${2:-}"
      shift 2
      ;;
    --state-file)
      STATE_FILE="${2:-}"
      shift 2
      ;;
    --keep-alive)
      KEEP_ALIVE=1
      shift
      ;;
    --)
      shift
      PASSTHROUGH_ARGS=("$@")
      break
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      PASSTHROUGH_ARGS+=("$1")
      shift
      ;;
  esac
done

if [[ "$(uname -s)" == "Darwin" && -z "${CN_API_KEY:-}" ]]; then
  echo "error: CN_API_KEY is required on macOS for desktop automation." >&2
  exit 1
fi

if [[ -z "${STATE_FILE}" ]]; then
  STATE_FILE="$(mktemp /tmp/ctx-remote-fixture-state.XXXXXX)"
fi

cleanup() {
  if [[ "${KEEP_ALIVE}" == "1" ]]; then
    return 0
  fi
  "${FIXTURE_SCRIPT}" stop --state-file "${STATE_FILE}" >/dev/null 2>&1 || true
}
trap cleanup EXIT

START_CMD=("${FIXTURE_SCRIPT}" start --runtime "${RUNTIME}" --state-file "${STATE_FILE}")
if [[ -n "${LOG_DIR}" ]]; then
  START_CMD+=(--log-dir "${LOG_DIR}")
fi
fixture_exports="$("${START_CMD[@]}")"
eval "${fixture_exports}"

# Do not override HOME here. Cargo/rustup and Tauri tooling invoked by automation
# rely on the host HOME for toolchain discovery in CI/local environments.
export CTX_DESKTOP_SSH_CONFIG_PATH="${CTX_AUTOMATION_REMOTE_FIXTURE_SSH_CONFIG}"
export CTX_AUTOMATION_SSH_NO_START_REMOTE="${CTX_AUTOMATION_SSH_NO_START_REMOTE:-0}"
export CTX_AUTOMATION_SKIP_REMOTE_CTX_PROVISION="${CTX_AUTOMATION_SKIP_REMOTE_CTX_PROVISION:-1}"
export CTX_AUTOMATION_USE_EXTERNAL_DAEMON="${CTX_AUTOMATION_USE_EXTERNAL_DAEMON:-0}"
export CTX_AUTOMATION_REMOTE_PASSWORD=""

if [[ "${KEEP_ALIVE}" == "1" ]]; then
  echo "fixture kept alive; state file: ${STATE_FILE}" >&2
  echo "stop with: ${FIXTURE_SCRIPT} stop --state-file ${STATE_FILE}" >&2
fi

cd "${ROOT}"
if [[ ${#PASSTHROUGH_ARGS[@]} -gt 0 ]]; then
  pnpm -C core/apps/desktop test:automation:remote-bootstrap "${PASSTHROUGH_ARGS[@]}"
else
  pnpm -C core/apps/desktop test:automation:remote-bootstrap
fi
