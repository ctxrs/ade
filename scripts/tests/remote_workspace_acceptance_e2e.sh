#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

MODE="${1:-fresh}"
case "$MODE" in
  fresh|upgrade) ;;
  *)
    echo "error: unsupported remote workspace acceptance mode '$MODE' (expected fresh or upgrade)" >&2
    exit 2
    ;;
esac

REMOTE_HOST="${CTX_AUTOMATION_REMOTE_HOST:-${CTX_UPDATER_E2E_REMOTE_HOST:-}}"
REMOTE_USER="${CTX_AUTOMATION_REMOTE_USER:-${CTX_UPDATER_E2E_REMOTE_USER:-root}}"
REMOTE_KEY="${CTX_AUTOMATION_REMOTE_SSH_KEY_PATH:-${CTX_UPDATER_E2E_SSH_KEY_PATH:-}}"
REMOTE_SSH_PORT="${CTX_AUTOMATION_REMOTE_SSH_PORT:-}"
REMOTE_DATA_DIR="${CTX_AUTOMATION_REMOTE_DATA_DIR:-/tmp/ctx-remote-workspace-e2e/daemon}"
REMOTE_CTX_BIN="${CTX_UPDATER_E2E_REMOTE_CTX_BIN:-/tmp/ctx-e2e-bin/ctx}"
ARTIFACT_ROOT="${CTX_REMOTE_CI_ARTIFACT_DIR:-${ROOT}/core/apps/desktop/automation/artifacts/remote-workspace-e2e/${MODE}}"
CTX_REMOTE_WORKSPACE_E2E_RUN_CONTAINER="${CTX_REMOTE_WORKSPACE_E2E_RUN_CONTAINER:-0}"

if [[ -z "$REMOTE_HOST" ]]; then
  echo "error: CTX_AUTOMATION_REMOTE_HOST/CTX_UPDATER_E2E_REMOTE_HOST is required" >&2
  exit 2
fi
if [[ -z "$REMOTE_KEY" ]]; then
  echo "error: CTX_AUTOMATION_REMOTE_SSH_KEY_PATH/CTX_UPDATER_E2E_SSH_KEY_PATH is required" >&2
  exit 2
fi
case "$CTX_REMOTE_WORKSPACE_E2E_RUN_CONTAINER" in
  0|1) ;;
  *)
    echo "error: CTX_REMOTE_WORKSPACE_E2E_RUN_CONTAINER must be 0 or 1" >&2
    exit 2
    ;;
esac

mkdir -p "$ARTIFACT_ROOT"

cleanup_pids=()
cleanup_ports=()

add_cleanup_port() {
  local port="${1:-}"
  if ! [[ "$port" =~ ^[0-9]+$ ]]; then
    return 0
  fi
  local existing
  for existing in "${cleanup_ports[@]}"; do
    if [[ "$existing" == "$port" ]]; then
      return 0
    fi
  done
  cleanup_ports+=("$port")
}

extract_webdriver_port() {
  local cmd="$1"
  if [[ "$cmd" =~ --port=([0-9]+) ]]; then
    printf '%s\n' "${BASH_REMATCH[1]}"
    return 0
  fi
  if [[ "$cmd" =~ --port[[:space:]]+([0-9]+) ]]; then
    printf '%s\n' "${BASH_REMATCH[1]}"
    return 0
  fi
  return 1
}

cleanup_local_desktop_automation_processes() {
  if ! command -v ps >/dev/null 2>&1; then
    return 0
  fi

  cleanup_pids=()
  cleanup_ports=()
  add_cleanup_port "${TAURI_DRIVER_PORT:-}"
  local pid cmd port
  while read -r pid cmd; do
    if [[ -z "${pid}" || -z "${cmd:-}" ]]; then
      continue
    fi
    case "${cmd}" in
      *WebKitWebDriver*|*wkwebdriver*|*WebKitWebProcess*|*WebKitNetworkProcess*|*WebKitGPUProcess*|*WebKitPluginProcess*|*WebKitStorageProcess*|*WebKitWebExtension*)
        cleanup_pids+=("${pid}")
        if port="$(extract_webdriver_port "$cmd")"; then
          add_cleanup_port "$port"
        fi
        ;;
      *"ctx-daemon serve "*"ctx-desktop-e2e-app-daemon-"*|*"ctx-daemon serve "*"remote-workspace-e2e"*)
        cleanup_pids+=("${pid}")
        ;;
      *Xvfb*".ctx/volatile/artifacts/ctx-desktop-e2e"*|*Xvfb*"remote-workspace-e2e"*)
        cleanup_pids+=("${pid}")
        ;;
    esac
  done < <(ps -Ao pid=,command= 2>/dev/null || true)

  if [[ "${#cleanup_pids[@]}" -gt 0 ]]; then
    kill -9 "${cleanup_pids[@]}" >/dev/null 2>&1 || true
  fi
}

wait_for_local_pids_gone() {
  local attempts="${1:-50}"
  shift || true
  if [[ "$#" -eq 0 ]]; then
    return 0
  fi
  local attempt=1
  local pid alive
  while [[ "$attempt" -le "$attempts" ]]; do
    alive=0
    for pid in "$@"; do
      if kill -0 "$pid" >/dev/null 2>&1; then
        alive=1
        break
      fi
    done
    if [[ "$alive" -eq 0 ]]; then
      return 0
    fi
    sleep 0.2
    attempt=$((attempt + 1))
  done
  return 1
}

wait_for_local_port_closed() {
  local port="$1"
  local attempts="${2:-50}"
  local attempt=1
  while [[ "$attempt" -le "$attempts" ]]; do
    if ! (true >/dev/tcp/127.0.0.1/"$port") >/dev/null 2>&1; then
      return 0
    fi
    sleep 0.2
    attempt=$((attempt + 1))
  done
  return 1
}

cleanup_local_desktop_automation() {
  cleanup_local_desktop_automation_processes
  wait_for_local_pids_gone 50 "${cleanup_pids[@]}" || true
  local port
  for port in "${cleanup_ports[@]}"; do
    wait_for_local_port_closed "$port" 50 || true
  done
}

trap cleanup_local_desktop_automation EXIT
cleanup_local_desktop_automation

ssh_base_args=(
  -i "$REMOTE_KEY"
  -F /dev/null
  -o BatchMode=yes
  -o StrictHostKeyChecking=no
  -o UserKnownHostsFile=/dev/null
  -o ConnectTimeout=12
)
if [[ -n "$REMOTE_SSH_PORT" ]]; then
  ssh_base_args+=(-p "$REMOTE_SSH_PORT")
fi

remote_ssh() {
  local command="$1"
  ssh "${ssh_base_args[@]}" "${REMOTE_USER}@${REMOTE_HOST}" "bash -lc $(printf '%q' "$command")"
}

prepare_remote_host() {
  local deps="${CTX_REMOTE_WORKSPACE_E2E_REMOTE_DEPS:-ca-certificates curl git jq xz-utils procps psmisc}"
  if [[ -z "$deps" ]]; then
    return 0
  fi
  local install_cmd
  install_cmd="$(cat <<EOF_CMD
set -euo pipefail
if command -v apt-get >/dev/null 2>&1; then
  export DEBIAN_FRONTEND=noninteractive
  apt-get update >/dev/null
  apt-get install -y --no-install-recommends ${deps} >/dev/null
fi
EOF_CMD
)"
  remote_ssh "$install_cmd"
}

reset_remote_for_fresh_install() {
  local remote_ctx_bin_q remote_data_dir_q remote_ctx_parent_q remote_data_parent_q
  remote_ctx_bin_q="$(printf '%q' "$REMOTE_CTX_BIN")"
  remote_data_dir_q="$(printf '%q' "$REMOTE_DATA_DIR")"
  remote_ctx_parent_q="$(printf '%q' "$(dirname "$REMOTE_CTX_BIN")")"
  remote_data_parent_q="$(printf '%q' "$(dirname "$REMOTE_DATA_DIR")")"
  local reset_cmd
  reset_cmd="$(cat <<EOF_CMD
set -euo pipefail
if command -v pkill >/dev/null 2>&1; then
  pkill -x ctx >/dev/null 2>&1 || true
fi
rm -f ${remote_ctx_bin_q} "\$HOME/.ctx/bin/ctx"
rm -rf ${remote_data_dir_q}
mkdir -p ${remote_ctx_parent_q} ${remote_data_parent_q} "\$HOME/.ctx/bin"
EOF_CMD
)"
  remote_ssh "$reset_cmd"
}

run_upgrade_proof() {
  local proof_dir="${ARTIFACT_ROOT}/updater-proof"
  mkdir -p "$proof_dir"
  CTX_REMOTE_CI_ARTIFACT_DIR="$proof_dir" \
    CTX_UPDATER_REMOTE_E2E_CONTROLLER_LAUNCH_SMOKE=0 \
    CTX_UPDATER_REMOTE_PROOF_IDLE="${CTX_UPDATER_REMOTE_PROOF_IDLE:-1}" \
    CTX_UPDATER_REMOTE_PROOF_INCOMPATIBLE_RECONNECT="${CTX_UPDATER_REMOTE_PROOF_INCOMPATIBLE_RECONNECT:-1}" \
    CTX_UPDATER_REMOTE_PROOF_NO_CLIENT_AUTO="${CTX_UPDATER_REMOTE_PROOF_NO_CLIENT_AUTO:-1}" \
    CTX_UPDATER_REMOTE_PROOF_PENDING_IDLE="${CTX_UPDATER_REMOTE_PROOF_PENDING_IDLE:-0}" \
    CTX_UPDATER_REMOTE_PROOF_PENDING_RESTART_NOW="${CTX_UPDATER_REMOTE_PROOF_PENDING_RESTART_NOW:-0}" \
    scripts/tests/updater_remote_daemon_e2e.sh
}

prepare_remote_host

case "$MODE" in
  fresh)
    reset_remote_for_fresh_install
    export CTX_AUTOMATION_REMOTE_SKIP_MANAGED_BINARY_RESET="${CTX_AUTOMATION_REMOTE_SKIP_MANAGED_BINARY_RESET:-0}"
    ;;
  upgrade)
    run_upgrade_proof
    export CTX_AUTOMATION_REMOTE_SKIP_MANAGED_BINARY_RESET=1
    ;;
esac

export CTX_AUTOMATION_REMOTE_HOST="$REMOTE_HOST"
export CTX_AUTOMATION_REMOTE_USER="$REMOTE_USER"
export CTX_AUTOMATION_REMOTE_DATA_DIR="$REMOTE_DATA_DIR"
export CTX_AUTOMATION_REMOTE_SSH_KEY_PATH="$REMOTE_KEY"
export CTX_AUTOMATION_REMOTE_CONTAINER_HOST="${CTX_AUTOMATION_REMOTE_CONTAINER_HOST:-$REMOTE_HOST}"
export CTX_AUTOMATION_REMOTE_CONTAINER_USER="${CTX_AUTOMATION_REMOTE_CONTAINER_USER:-$REMOTE_USER}"
export CTX_AUTOMATION_REMOTE_CONTAINER_DATA_DIR="${CTX_AUTOMATION_REMOTE_CONTAINER_DATA_DIR:-${REMOTE_DATA_DIR}-sandbox}"
export CTX_AUTOMATION_REMOTE_CONTAINER_SSH_KEY_PATH="${CTX_AUTOMATION_REMOTE_CONTAINER_SSH_KEY_PATH:-$REMOTE_KEY}"
# Remote workspace acceptance owns remote bootstrap, managed binary install,
# workspace launch, and provider-auth readiness. Live provider completion is
# advisory here because provider-specific gates own hard first-turn coverage.
export CTX_AUTOMATION_REMOTE_REQUIRE_FIRST_TURN_SUCCESS="${CTX_AUTOMATION_REMOTE_REQUIRE_FIRST_TURN_SUCCESS:-0}"
export CTX_REMOTE_WORKSPACE_E2E_REQUIRE_ALL_HARNESS_INSTALLS="${CTX_REMOTE_WORKSPACE_E2E_REQUIRE_ALL_HARNESS_INSTALLS:-1}"

core/apps/desktop/scripts/test_remote_real_ci.sh \
  --artifacts-dir "${ARTIFACT_ROOT}/remote-contracts" \
  --run-host 1 \
  --run-container "${CTX_REMOTE_WORKSPACE_E2E_RUN_CONTAINER}"
