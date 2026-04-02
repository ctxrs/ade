#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")"/../../../.. && pwd)"
FIXTURE_SCRIPT="${ROOT}/core/apps/desktop/scripts/remote_ssh_fixture.sh"
RUNNER_SCRIPT="${ROOT}/core/apps/desktop/scripts/test_remote_real_ci.sh"
RELEASE_FIXTURE_SCRIPT="${ROOT}/core/apps/desktop/scripts/local_release_fixture.cjs"
LOCAL_RELEASE_APP_PATH="${ROOT}/core/target/release/bundle/macos/ctx.app"

RUNTIME="auto"
RUN_HOST="${CTX_REMOTE_CI_RUN_HOST:-0}"
AUTH_MODE="${CTX_AUTOMATION_REMOTE_FIXTURE_AUTH_MODE:-key}"
FIXTURE_PASSWORD="${CTX_AUTOMATION_REMOTE_FIXTURE_PASSWORD:-}"
KEEP_ALIVE=0
LOG_DIR=""
STATE_FILE="${CTX_AUTOMATION_REMOTE_FIXTURE_STATE_FILE:-}"
RELEASE_FIXTURE_STATE_FILE="${CTX_AUTOMATION_REMOTE_RELEASE_FIXTURE_STATE_FILE:-}"
ARTIFACTS_DIR=""
RUN_CONTAINER="${CTX_REMOTE_CI_RUN_CONTAINER:-1}"
PASSTHROUGH_ARGS=()

usage() {
  cat <<'USAGE' >&2
usage:
  test_remote_docker_contracts.sh [--runtime auto|docker|nerdctl] [--auth-mode key|password] [--password VALUE] [--log-dir PATH] [--state-file PATH] [--artifacts-dir PATH] [--run-container 1|0] [--keep-alive] [-- ...runner args]

notes:
  - Starts the local Docker SSH fixture and exports both host and sandbox lane env vars.
  - Starts a local managed-release mirror from repo-built linux daemon artifacts when CTX_DOWNLOAD_BASE_URL is unset.
  - On macOS, also builds/uses a release automation app bundle so WDIO does not fall back to a separate debug app build.
  - Defaults `CTX_AUTOMATION_REMOTE_FIXTURE_CLASS=docker-ssh`.
  - Defaults `CTX_AUTOMATION_REMOTE_FIXTURE_SANDBOX_RUNTIME=nested-containerd`.
  - Defaults `CTX_REMOTE_CI_RUN_HOST=0` because the real remote+sandbox wizard lane now resets the remote host and proves bootstrap/install itself.
  - Defaults `CTX_AUTOMATION_REMOTE_REQUIRE_FIRST_TURN_SUCCESS=1` so the sandbox lane proves a usable workspace, not only creation.
USAGE
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --runtime)
      RUNTIME="${2:-}"
      shift 2
      ;;
    --auth-mode)
      AUTH_MODE="${2:-}"
      shift 2
      ;;
    --password)
      FIXTURE_PASSWORD="${2:-}"
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
    --artifacts-dir)
      ARTIFACTS_DIR="${2:-}"
      shift 2
      ;;
    --run-container)
      RUN_CONTAINER="${2:-}"
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
  echo "error: CN_API_KEY is required on macOS for desktop automation (stored in Infisical for core/)." >&2
  exit 1
fi

if [[ "${RUN_CONTAINER}" != "0" && "${RUN_CONTAINER}" != "1" ]]; then
  echo "error: --run-container must be 1 or 0" >&2
  exit 2
fi
if [[ "${RUN_HOST}" != "0" && "${RUN_HOST}" != "1" ]]; then
  echo "error: CTX_REMOTE_CI_RUN_HOST must be 1 or 0" >&2
  exit 2
fi

if [[ -z "${STATE_FILE}" ]]; then
  STATE_FILE="$(mktemp /tmp/ctx-remote-fixture-state.XXXXXX)"
fi

cleanup() {
  if [[ "${KEEP_ALIVE}" == "1" ]]; then
    return 0
  fi
  if [[ -n "${RELEASE_FIXTURE_STATE_FILE}" ]]; then
    node "${RELEASE_FIXTURE_SCRIPT}" stop --state-file "${RELEASE_FIXTURE_STATE_FILE}" >/dev/null 2>&1 || true
  fi
  "${FIXTURE_SCRIPT}" stop --state-file "${STATE_FILE}" >/dev/null 2>&1 || true
}
trap cleanup EXIT

if [[ -z "${CTX_DOWNLOAD_BASE_URL:-}" && "${CTX_AUTOMATION_REMOTE_USE_LOCAL_RELEASE_FIXTURE:-1}" == "1" ]]; then
  if [[ "${CTX_AUTOMATION_SKIP_DESKTOP_PREP_RELEASE:-0}" != "1" ]]; then
    echo "[remote-contracts] preparing release resources for local managed-daemon mirror" >&2
    export CTX_DESKTOP_ALLOW_MANAGED_AVF_RUNTIME_MISSING_LOCAL_PAYLOAD=1
    pnpm -C "${ROOT}/core" desktop:prep:release
    export CTX_AUTOMATION_SKIP_DESKTOP_PREP_RELEASE=1
  fi
  if [[ "$(uname -s)" == "Darwin" && -z "${CTX_DESKTOP_APP_PATH:-}" ]]; then
    echo "[remote-contracts] building release automation app bundle for WDIO" >&2
    CTX_DESKTOP_SYNC_BUNDLES=0 \
    CTX_BUNDLE_REMOTE_DAEMONS=0 \
    CTX_DESKTOP_ALLOW_MANAGED_AVF_RUNTIME_MISSING_LOCAL_PAYLOAD=1 \
    pnpm -C "${ROOT}/core/apps/desktop" run build -- --bundles app -- --features automation
    export CTX_DESKTOP_APP_PATH="${LOCAL_RELEASE_APP_PATH}"
  fi
  if [[ -z "${RELEASE_FIXTURE_STATE_FILE}" ]]; then
    if [[ -n "${ARTIFACTS_DIR}" ]]; then
      mkdir -p "${ARTIFACTS_DIR}"
      RELEASE_FIXTURE_STATE_FILE="${ARTIFACTS_DIR}/local-release-fixture.state"
    else
      mkdir -p "${ROOT}/core/apps/desktop/automation/artifacts"
      RELEASE_FIXTURE_STATE_FILE="$(mktemp "${ROOT}/core/apps/desktop/automation/artifacts/local-release-fixture.state.XXXXXX")"
    fi
  fi
  release_exports="$(node "${RELEASE_FIXTURE_SCRIPT}" start --state-file "${RELEASE_FIXTURE_STATE_FILE}" --channel "${CTX_DESKTOP_CHANNEL:-stable}")"
  eval "${release_exports}"
  export CTX_DESKTOP_ALLOW_INSECURE_LOCAL_UPDATER_FOR_REMOTE_BOOTSTRAP=1
fi

START_CMD=(
  "${FIXTURE_SCRIPT}" start
  --runtime "${RUNTIME}"
  --state-file "${STATE_FILE}"
  --auth-mode "${AUTH_MODE}"
  --export-container-lane
)
if [[ -n "${FIXTURE_PASSWORD}" ]]; then
  START_CMD+=(--password "${FIXTURE_PASSWORD}")
fi
if [[ -n "${LOG_DIR}" ]]; then
  START_CMD+=(--log-dir "${LOG_DIR}")
fi
fixture_exports="$("${START_CMD[@]}")"
eval "${fixture_exports}"

export CTX_DESKTOP_SSH_CONFIG_PATH="${CTX_AUTOMATION_REMOTE_FIXTURE_SSH_CONFIG}"
export CTX_AUTOMATION_SSH_NO_START_REMOTE="${CTX_AUTOMATION_SSH_NO_START_REMOTE:-0}"
export CTX_AUTOMATION_SKIP_REMOTE_CTX_PROVISION="${CTX_AUTOMATION_SKIP_REMOTE_CTX_PROVISION:-1}"
export CTX_AUTOMATION_USE_EXTERNAL_DAEMON="${CTX_AUTOMATION_USE_EXTERNAL_DAEMON:-0}"
export CTX_AUTOMATION_ALLOW_PREP_APP_PROCESS_SWEEP="${CTX_AUTOMATION_ALLOW_PREP_APP_PROCESS_SWEEP:-1}"
export CTX_AUTOMATION_REMOTE_FIXTURE_CLASS="${CTX_AUTOMATION_REMOTE_FIXTURE_CLASS:-docker-ssh}"
export CTX_AUTOMATION_REMOTE_FIXTURE_SANDBOX_RUNTIME="${CTX_AUTOMATION_REMOTE_FIXTURE_SANDBOX_RUNTIME:-nested-containerd}"
export CTX_AUTOMATION_REMOTE_STRICT="${CTX_AUTOMATION_REMOTE_STRICT:-1}"
export CTX_AUTOMATION_REMOTE_REQUIRE_FIRST_TURN_SUCCESS="${CTX_AUTOMATION_REMOTE_REQUIRE_FIRST_TURN_SUCCESS:-1}"
if [[ -n "${CTX_DESKTOP_APP_PATH:-}" && -z "${CTX_AUTOMATION_SKIP_APP_BUILD+x}" ]]; then
  export CTX_AUTOMATION_SKIP_APP_BUILD=1
fi
if [[ -z "${CTX_AUTOMATION_REMOTE_ALLOW_SKIP+x}" ]]; then
  export CTX_AUTOMATION_REMOTE_ALLOW_SKIP=0
fi

if [[ "${KEEP_ALIVE}" == "1" ]]; then
  echo "fixture kept alive; state file: ${STATE_FILE}" >&2
  echo "stop with: ${FIXTURE_SCRIPT} stop --state-file ${STATE_FILE}" >&2
  if [[ -n "${RELEASE_FIXTURE_STATE_FILE}" ]]; then
    echo "local release fixture state file: ${RELEASE_FIXTURE_STATE_FILE}" >&2
    echo "stop with: node ${RELEASE_FIXTURE_SCRIPT} stop --state-file ${RELEASE_FIXTURE_STATE_FILE}" >&2
  fi
fi

CMD=("${RUNNER_SCRIPT}" "--run-container" "${RUN_CONTAINER}")
CMD+=("--run-host" "${RUN_HOST}")
if [[ -n "${ARTIFACTS_DIR}" ]]; then
  CMD+=(--artifacts-dir "${ARTIFACTS_DIR}")
fi
if [[ ${#PASSTHROUGH_ARGS[@]} -gt 0 ]]; then
  CMD+=("${PASSTHROUGH_ARGS[@]}")
fi

cd "${ROOT}"
"${CMD[@]}"
