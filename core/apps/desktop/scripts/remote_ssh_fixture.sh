#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
FIXTURE_DIR="${SCRIPT_DIR}/remote-fixture"

usage() {
  cat <<'USAGE' >&2
usage:
  remote_ssh_fixture.sh start [--runtime auto|docker|nerdctl] [--auth-mode key|password] [--password VALUE] [--state-file PATH] [--log-dir PATH] [--user NAME] [--export-container-lane]
  remote_ssh_fixture.sh stop [--state-file PATH]
  remote_ssh_fixture.sh print-env [--state-file PATH]

notes:
  - `start` prints `export ...` lines to stdout. Use with `eval "$(... start ...)"`.
  - `stop` removes the fixture container and temp files from the state file.
USAGE
}

die() {
  echo "error: $*" >&2
  exit 1
}

log() {
  echo "[remote_ssh_fixture] $*" >&2
}

quote_export() {
  local key="$1"
  local value="$2"
  printf 'export %s=%q\n' "$key" "$value"
}

save_state_var() {
  local key="$1"
  local value="$2"
  printf '%s=%q\n' "$key" "$value" >>"$STATE_FILE"
}

ensure_exists() {
  local cmd="$1"
  command -v "$cmd" >/dev/null 2>&1 || die "required command not found: ${cmd}"
}

run_with_timeout() {
  local seconds="$1"
  shift
  "$@" &
  local pid=$!
  (
    sleep "${seconds}"
    if kill -0 "${pid}" >/dev/null 2>&1; then
      kill -TERM "${pid}" >/dev/null 2>&1 || true
      sleep 1
      kill -KILL "${pid}" >/dev/null 2>&1 || true
    fi
  ) &
  local watchdog_pid=$!
  local status=0
  if wait "${pid}"; then
    status=0
  else
    status=$?
  fi
  kill -KILL "${watchdog_pid}" >/dev/null 2>&1 || true
  wait "${watchdog_pid}" >/dev/null 2>&1 || true
  return "${status}"
}

runtime_healthy() {
  local runtime="$1"
  run_with_timeout 8 "${runtime}" ps >/dev/null 2>&1
}

pick_runtime() {
  local requested="$1"
  case "$requested" in
    docker|nerdctl)
      ensure_exists "$requested"
      runtime_healthy "$requested" || die "${requested} is installed but not responding"
      echo "$requested"
      return 0
      ;;
    auto)
      if command -v docker >/dev/null 2>&1; then
        if runtime_healthy docker; then
          echo "docker"
          return 0
        fi
        log "docker is installed but not responding; trying nerdctl"
      fi
      if command -v nerdctl >/dev/null 2>&1; then
        if runtime_healthy nerdctl; then
          echo "nerdctl"
          return 0
        fi
        log "nerdctl is installed but not responding"
      fi
      ;;
  esac
  die "no supported container runtime found (tried docker, nerdctl)"
}

default_fixture_image_tag() {
  local checksum
  checksum="$(
    cksum "${FIXTURE_DIR}/Dockerfile" "${FIXTURE_DIR}/entrypoint.sh" \
      | awk '{print $1}' \
      | tr '\n' '-' \
      | sed 's/-$//'
  )"
  printf 'ctx-remote-ssh-fixture:local-%s' "${checksum}"
}

parse_flags() {
  RUNTIME="${CTX_AUTOMATION_REMOTE_FIXTURE_RUNTIME:-auto}"
  AUTH_MODE="${CTX_AUTOMATION_REMOTE_FIXTURE_AUTH_MODE:-key}"
  FIXTURE_PASSWORD="${CTX_AUTOMATION_REMOTE_FIXTURE_PASSWORD:-}"
  PRESEED_KEY="${CTX_AUTOMATION_REMOTE_FIXTURE_PRESEED_KEY:-}"
  STATE_FILE="${CTX_AUTOMATION_REMOTE_FIXTURE_STATE_FILE:-}"
  LOG_DIR="${CTX_AUTOMATION_REMOTE_FIXTURE_LOG_DIR:-}"
  FIXTURE_USER="${CTX_AUTOMATION_REMOTE_FIXTURE_USER:-ctxfixture}"
  DAEMON_PORT="${CTX_AUTOMATION_REMOTE_FIXTURE_DAEMON_PORT:-44099}"
  IMAGE_TAG="${CTX_AUTOMATION_REMOTE_FIXTURE_IMAGE:-$(default_fixture_image_tag)}"
  EXPORT_CONTAINER_LANE="${CTX_AUTOMATION_REMOTE_FIXTURE_EXPORT_CONTAINER_LANE:-0}"

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
      --state-file)
        STATE_FILE="${2:-}"
        shift 2
        ;;
      --log-dir)
        LOG_DIR="${2:-}"
        shift 2
        ;;
      --user)
        FIXTURE_USER="${2:-}"
        shift 2
        ;;
      --export-container-lane)
        EXPORT_CONTAINER_LANE="1"
        shift
        ;;
      -h|--help)
        usage
        exit 0
        ;;
      *)
        die "unknown flag: $1"
        ;;
    esac
  done
}

load_state() {
  [[ -n "${STATE_FILE}" ]] || die "--state-file is required for this command"
  [[ -f "${STATE_FILE}" ]] || die "state file not found: ${STATE_FILE}"
  # shellcheck disable=SC1090
  source "${STATE_FILE}"
}

container_port() {
  local runtime="$1"
  local container="$2"
  local output
  output="$("${runtime}" port "${container}" 22/tcp 2>/dev/null | head -n 1 | tr -d '\r')"
  [[ -n "${output}" ]] || return 1
  printf '%s' "${output}" | sed -E 's/.*:([0-9]+)$/\1/'
}

render_exports_from_loaded_state() {
  quote_export CTX_AUTOMATION_REMOTE_HOST "127.0.0.1"
  quote_export CTX_AUTOMATION_REMOTE_FIXTURE_HOST_ALIAS "${FIXTURE_HOST_ALIAS}"
  quote_export CTX_AUTOMATION_REMOTE_USER "${FIXTURE_USER}"
  quote_export CTX_AUTOMATION_REMOTE_PORT "${FIXTURE_DAEMON_PORT}"
  quote_export CTX_AUTOMATION_REMOTE_DATA_DIR "${FIXTURE_REMOTE_DATA_DIR}"
  quote_export CTX_AUTOMATION_REMOTE_PASSWORD "${FIXTURE_PASSWORD}"
  quote_export CTX_AUTOMATION_REMOTE_PASSWORD_ACTUAL "${FIXTURE_PASSWORD}"
  quote_export CTX_AUTOMATION_REMOTE_AUTH_MODE "${FIXTURE_AUTH_MODE}"
  quote_export CTX_AUTOMATION_REMOTE_SSH_KEY_PATH "${FIXTURE_KEY_PATH}"
  quote_export CTX_UPDATER_E2E_SSH_KEY_PATH "${FIXTURE_KEY_PATH}"
  quote_export CTX_AUTOMATION_REMOTE_FIXTURE_HOME "${FIXTURE_SSH_HOME}"
  quote_export CTX_AUTOMATION_REMOTE_FIXTURE_SSH_PORT "${FIXTURE_HOST_PORT}"
  quote_export CTX_AUTOMATION_REMOTE_SSH_PORT "${FIXTURE_HOST_PORT}"
  quote_export CTX_AUTOMATION_REMOTE_FIXTURE_SSH_CONFIG "${FIXTURE_SSH_CONFIG}"
  quote_export CTX_AUTOMATION_REMOTE_FIXTURE_STATE_FILE "${STATE_FILE}"
  quote_export CTX_AUTOMATION_REMOTE_FIXTURE_RUNTIME "${FIXTURE_RUNTIME}"
  quote_export CTX_AUTOMATION_REMOTE_FIXTURE_LOG_DIR "${FIXTURE_LOG_DIR}"
  if [[ "${FIXTURE_EXPORT_CONTAINER_LANE:-0}" == "1" ]]; then
    quote_export CTX_AUTOMATION_REMOTE_CONTAINER_HOST "127.0.0.1"
    quote_export CTX_AUTOMATION_REMOTE_CONTAINER_USER "${FIXTURE_USER}"
    quote_export CTX_AUTOMATION_REMOTE_CONTAINER_PORT "${FIXTURE_DAEMON_PORT}"
    quote_export CTX_AUTOMATION_REMOTE_CONTAINER_DATA_DIR "${FIXTURE_REMOTE_CONTAINER_DATA_DIR}"
    quote_export CTX_AUTOMATION_REMOTE_CONTAINER_PASSWORD "${FIXTURE_PASSWORD}"
    quote_export CTX_AUTOMATION_REMOTE_CONTAINER_PASSWORD_ACTUAL "${FIXTURE_PASSWORD}"
    quote_export CTX_AUTOMATION_REMOTE_CONTAINER_AUTH_MODE "${FIXTURE_AUTH_MODE}"
    quote_export CTX_AUTOMATION_REMOTE_CONTAINER_SSH_KEY_PATH "${FIXTURE_KEY_PATH}"
    quote_export CTX_AUTOMATION_REMOTE_CONTAINER_FIXTURE_SSH_CONFIG "${FIXTURE_SSH_CONFIG}"
    quote_export CTX_AUTOMATION_REMOTE_CONTAINER_SSH_PORT "${FIXTURE_HOST_PORT}"
  fi
}

wait_for_ssh_ready() {
  local config_path="$1"
  local alias="$2"
  local auth_mode="$3"
  local password="$4"
  local attempts="${5:-60}"
  local i
  for ((i = 1; i <= attempts; i += 1)); do
    if [[ "${auth_mode}" == "password" ]]; then
      if SSHPASS="${password}" sshpass -e ssh -F "${config_path}" \
        -o BatchMode=no \
        -o PreferredAuthentications=password,keyboard-interactive \
        -o NumberOfPasswordPrompts=1 \
        -o ConnectTimeout=2 \
        "${alias}" "echo ready" >/dev/null 2>&1; then
        return 0
      fi
    else
      if ssh -F "${config_path}" \
        -o BatchMode=yes \
        -o ConnectTimeout=2 \
        "${alias}" "echo ready" >/dev/null 2>&1; then
        return 0
      fi
    fi
    sleep 1
  done
  return 1
}

start_fixture() {
  ensure_exists ssh
  ensure_exists ssh-keygen
  ensure_exists mktemp

  case "${AUTH_MODE}" in
    key|password)
      ;;
    *)
      die "unsupported --auth-mode: ${AUTH_MODE} (expected key or password)"
      ;;
  esac
  if [[ "${AUTH_MODE}" == "password" ]]; then
    ensure_exists sshpass
    if [[ -z "${FIXTURE_PASSWORD}" ]]; then
      FIXTURE_PASSWORD="ctx-fixture-${RANDOM}-${RANDOM}-Pw!"
    fi
  else
    FIXTURE_PASSWORD=""
  fi
  if [[ -z "${PRESEED_KEY}" ]]; then
    if [[ "${AUTH_MODE}" == "key" ]]; then
      PRESEED_KEY="1"
    else
      PRESEED_KEY="0"
    fi
  fi

  local runtime
  runtime="$(pick_runtime "${RUNTIME}")"
  local tmp_dir
  tmp_dir="$(mktemp -d /tmp/ctx-remote-fixture.XXXXXX)"
  local state_file
  if [[ -n "${STATE_FILE}" ]]; then
    state_file="${STATE_FILE}"
  else
    state_file="$(mktemp /tmp/ctx-remote-fixture-state.XXXXXX)"
  fi
  STATE_FILE="${state_file}"

  local log_dir
  if [[ -n "${LOG_DIR}" ]]; then
    log_dir="${LOG_DIR}"
    mkdir -p "${log_dir}"
  else
    log_dir="${tmp_dir}/logs"
    mkdir -p "${log_dir}"
  fi

  local key_path="${tmp_dir}/id_ed25519"
  local key_pub_path="${tmp_dir}/id_ed25519.pub"
  local authorized_keys="${tmp_dir}/authorized_keys"
  local ssh_home="${tmp_dir}/ssh-home"
  local ssh_dir="${ssh_home}/.ssh"
  local ssh_config="${ssh_dir}/config"
  local host_alias="ctx-fixture-${RANDOM}-$$"
  local remote_data_dir="/tmp/ctx-e2e-remote-fixture-${RANDOM}/daemon"
  local remote_container_data_dir="${remote_data_dir}-sandbox"
  local container_name="ctx-remote-fixture-${RANDOM}-$$"
  local container_id=""
  local startup_ok=0

  cleanup_on_failure() {
    if [[ "${startup_ok:-0}" == "1" ]]; then
      return 0
    fi
    if [[ -n "${container_id:-}" ]]; then
      "${runtime:-docker}" rm -f "${container_id}" >/dev/null 2>&1 || true
    fi
    rm -rf "${tmp_dir:-}"
    rm -f "${STATE_FILE:-}"
  }
  trap cleanup_on_failure EXIT

  ssh-keygen -q -t ed25519 -N "" -f "${key_path}" >/dev/null
  cp "${key_pub_path}" "${authorized_keys}"
  local authorized_keys_b64
  authorized_keys_b64="$(base64 <"${authorized_keys}" | tr -d '\r\n')"
  local fixture_authorized_keys_b64=""
  if [[ "${PRESEED_KEY}" == "1" || "${PRESEED_KEY}" == "true" || "${PRESEED_KEY}" == "yes" ]]; then
    fixture_authorized_keys_b64="${authorized_keys_b64}"
  fi

  mkdir -p "${ssh_dir}"
  chmod 700 "${ssh_dir}"

  if ! "${runtime}" image inspect "${IMAGE_TAG}" >/dev/null 2>&1; then
    log "building fixture image (${IMAGE_TAG}) with ${runtime}"
    "${runtime}" build -t "${IMAGE_TAG}" "${FIXTURE_DIR}" \
      >"${log_dir}/runtime-build.log" 2>&1
  fi

  container_id="$("${runtime}" run -d --rm \
    --name "${container_name}" \
    --privileged \
    -p 127.0.0.1::22 \
    -e "CTX_FIXTURE_USER=${FIXTURE_USER}" \
    -e "CTX_FIXTURE_HOME=/home/${FIXTURE_USER}" \
    -e "CTX_FIXTURE_AUTH_MODE=${AUTH_MODE}" \
    -e "CTX_FIXTURE_PASSWORD=${FIXTURE_PASSWORD}" \
    -e "CTX_FIXTURE_AUTHORIZED_KEYS_B64=${fixture_authorized_keys_b64}" \
    "${IMAGE_TAG}")"

  local host_ssh_port
  host_ssh_port="$(container_port "${runtime}" "${container_id}" || true)"
  [[ "${host_ssh_port}" =~ ^[0-9]+$ ]] || die "failed to resolve fixture SSH port"

  cat >"${ssh_config}" <<EOF
Host ${host_alias}
  HostName 127.0.0.1
  Port ${host_ssh_port}
  User ${FIXTURE_USER}
  IdentityFile ${key_path}
  IdentitiesOnly yes
  BatchMode yes
  StrictHostKeyChecking no
  UserKnownHostsFile /dev/null
Host 127.0.0.1
  Port ${host_ssh_port}
  User ${FIXTURE_USER}
  IdentityFile ${key_path}
  IdentitiesOnly yes
  BatchMode yes
  StrictHostKeyChecking no
  UserKnownHostsFile /dev/null
EOF
  chmod 600 "${ssh_config}"

  if ! wait_for_ssh_ready "${ssh_config}" "${host_alias}" "${AUTH_MODE}" "${FIXTURE_PASSWORD}" 60; then
    "${runtime}" logs "${container_id}" >"${log_dir}/container.log" 2>&1 || true
    die "fixture ssh endpoint did not become ready (see ${log_dir}/container.log)"
  fi

  if [[ "${AUTH_MODE}" == "password" ]]; then
    SSHPASS="${FIXTURE_PASSWORD}" sshpass -e ssh -F "${ssh_config}" \
      -o BatchMode=no \
      -o PreferredAuthentications=password,keyboard-interactive \
      "${host_alias}" "mkdir -p '${remote_data_dir}' '${remote_container_data_dir}'" >/dev/null 2>&1
  else
    ssh -F "${ssh_config}" "${host_alias}" "mkdir -p '${remote_data_dir}' '${remote_container_data_dir}'" >/dev/null 2>&1
  fi

  : >"${STATE_FILE}"
  save_state_var FIXTURE_RUNTIME "${runtime}"
  save_state_var FIXTURE_CONTAINER_ID "${container_id}"
  save_state_var FIXTURE_CONTAINER_NAME "${container_name}"
  save_state_var FIXTURE_TMP_DIR "${tmp_dir}"
  save_state_var FIXTURE_KEY_PATH "${key_path}"
  save_state_var FIXTURE_HOST_ALIAS "${host_alias}"
  save_state_var FIXTURE_USER "${FIXTURE_USER}"
  save_state_var FIXTURE_HOST_PORT "${host_ssh_port}"
  save_state_var FIXTURE_DAEMON_PORT "${DAEMON_PORT}"
  save_state_var FIXTURE_REMOTE_DATA_DIR "${remote_data_dir}"
  save_state_var FIXTURE_REMOTE_CONTAINER_DATA_DIR "${remote_container_data_dir}"
  save_state_var FIXTURE_AUTH_MODE "${AUTH_MODE}"
  save_state_var FIXTURE_PASSWORD "${FIXTURE_PASSWORD}"
  save_state_var FIXTURE_SSH_HOME "${ssh_home}"
  save_state_var FIXTURE_SSH_CONFIG "${ssh_config}"
  save_state_var FIXTURE_LOG_DIR "${log_dir}"
  save_state_var FIXTURE_IMAGE_TAG "${IMAGE_TAG}"
  save_state_var FIXTURE_EXPORT_CONTAINER_LANE "${EXPORT_CONTAINER_LANE}"
  save_state_var FIXTURE_STATE_FILE "${STATE_FILE}"

  load_state
  render_exports_from_loaded_state
  startup_ok=1
  trap - EXIT
}

stop_fixture() {
  load_state
  "${FIXTURE_RUNTIME}" rm -f "${FIXTURE_CONTAINER_ID}" >/dev/null 2>&1 || true
  rm -rf "${FIXTURE_TMP_DIR}"
  rm -f "${STATE_FILE}"
  log "stopped fixture ${FIXTURE_CONTAINER_NAME}"
}

print_env() {
  load_state
  render_exports_from_loaded_state
}

main() {
  local command="${1:-}"
  [[ -n "${command}" ]] || {
    usage
    exit 1
  }
  shift || true
  parse_flags "$@"
  case "${command}" in
    start)
      start_fixture
      ;;
    stop)
      stop_fixture
      ;;
    print-env)
      print_env
      ;;
    -h|--help|help)
      usage
      ;;
    *)
      usage
      die "unknown command: ${command}"
      ;;
  esac
}

main "$@"
