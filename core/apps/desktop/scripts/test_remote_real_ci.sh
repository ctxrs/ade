#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")"/../../../.. && pwd)"
RUN_ID="$(date -u +%Y%m%dT%H%M%SZ)"
ARTIFACT_DIR="${CTX_REMOTE_CI_ARTIFACT_DIR:-${ROOT}/core/apps/desktop/automation/artifacts/remote-contracts/${RUN_ID}}"
RUN_HOST="${CTX_REMOTE_CI_RUN_HOST:-1}"
RUN_CONTAINER="${CTX_REMOTE_CI_RUN_CONTAINER:-1}"
STRICT_REQUIRED="${CTX_AUTOMATION_REMOTE_STRICT:-1}"
ALLOW_SKIP="${CTX_AUTOMATION_REMOTE_ALLOW_SKIP:-0}"
REQUIRE_FIRST_TURN_SUCCESS="${CTX_AUTOMATION_REMOTE_REQUIRE_FIRST_TURN_SUCCESS:-0}"
DRY_RUN=0

usage() {
  cat <<'USAGE'
usage:
  test_remote_real_ci.sh [--artifacts-dir DIR] [--run-host 1|0] [--run-container 1|0] [--dry-run]

Runs the remote host bootstrap contract and, optionally, the remote container
contract against the configured remote fixture contract.

important env:
  CTX_REMOTE_CI_ARTIFACT_DIR
  CTX_REMOTE_CI_RUN_HOST=1|0
  CTX_REMOTE_CI_RUN_CONTAINER=1|0
  CTX_AUTOMATION_REMOTE_STRICT=1|0
  CTX_AUTOMATION_REMOTE_ALLOW_SKIP=1|0
  CTX_AUTOMATION_REMOTE_REQUIRE_FIRST_TURN_SUCCESS=1|0
USAGE
}

if [[ $# -gt 0 && "$1" == "--" ]]; then
  shift
fi

while [[ $# -gt 0 ]]; do
  case "$1" in
    --artifacts-dir)
      if [[ $# -lt 2 ]]; then
        echo "error: --artifacts-dir requires a value" >&2
        exit 2
      fi
      ARTIFACT_DIR="$2"
      shift 2
      ;;
    --run-host)
      if [[ $# -lt 2 ]]; then
        echo "error: --run-host requires 1 or 0" >&2
        exit 2
      fi
      RUN_HOST="$2"
      shift 2
      ;;
    --run-container)
      if [[ $# -lt 2 ]]; then
        echo "error: --run-container requires 1 or 0" >&2
        exit 2
      fi
      RUN_CONTAINER="$2"
      shift 2
      ;;
    --dry-run)
      DRY_RUN=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "error: unknown argument: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

if [[ "${RUN_HOST}" != "0" && "${RUN_HOST}" != "1" ]]; then
  echo "error: --run-host must be 1 or 0" >&2
  exit 2
fi

if [[ "${RUN_CONTAINER}" != "0" && "${RUN_CONTAINER}" != "1" ]]; then
  echo "error: --run-container must be 1 or 0" >&2
  exit 2
fi

mkdir -p "${ARTIFACT_DIR}"

if [[ -z "${CTX_DESKTOP_SSH_CONFIG_PATH:-}" ]]; then
  if [[ -n "${CTX_AUTOMATION_REMOTE_CONTAINER_FIXTURE_SSH_CONFIG:-}" ]]; then
    export CTX_DESKTOP_SSH_CONFIG_PATH="${CTX_AUTOMATION_REMOTE_CONTAINER_FIXTURE_SSH_CONFIG}"
  elif [[ -n "${CTX_AUTOMATION_REMOTE_FIXTURE_SSH_CONFIG:-}" ]]; then
    export CTX_DESKTOP_SSH_CONFIG_PATH="${CTX_AUTOMATION_REMOTE_FIXTURE_SSH_CONFIG}"
  fi
fi

HOST_DIR="${ARTIFACT_DIR}/remote-host"
CONTAINER_DIR="${ARTIFACT_DIR}/remote-container"
PRECHECK_JSON="${ARTIFACT_DIR}/preflight.json"
SUMMARY_TSV="${ARTIFACT_DIR}/summary.tsv"
SUMMARY_TXT="${ARTIFACT_DIR}/remote-real-ci-summary.txt"

mkdir -p "${HOST_DIR}"
if [[ "${RUN_CONTAINER}" == "1" ]]; then
  mkdir -p "${CONTAINER_DIR}"
fi

echo "[remote-contracts] artifact dir: ${ARTIFACT_DIR}" >&2

run_preflight() {
  if ! CTX_AUTOMATION_REMOTE_STRICT="${STRICT_REQUIRED}" \
    CTX_AUTOMATION_REMOTE_ALLOW_SKIP="${ALLOW_SKIP}" \
    node - "${ROOT}" "${RUN_CONTAINER}" >"${PRECHECK_JSON}" <<'NODE'
const path = require("node:path");
const root = process.argv[2];
const runContainer = process.argv[3] === "1";
const { resolveRemoteFixtureEnv } = require(path.join(root, "core/apps/desktop/automation/helpers/remote_fixture_contract.cjs"));

const host = resolveRemoteFixtureEnv({ lane: "host" });
const container = runContainer ? resolveRemoteFixtureEnv({ lane: "sandbox" }) : null;
const payload = {
  strict_required: host.strictRequired || Boolean(container && container.strictRequired),
  allow_skip: host.allowSkip || Boolean(container && container.allowSkip),
  lanes: {
    host,
    container,
  },
};
process.stdout.write(`${JSON.stringify(payload, null, 2)}\n`);

const failures = [];
if (!host.ready && host.strictRequired) {
  failures.push(host.preflightMessage);
}
if (container && !container.ready && container.strictRequired) {
  failures.push(container.preflightMessage);
}

if (failures.length > 0) {
  for (const failure of failures) {
    process.stderr.write(`[remote-contracts] ${failure}\n`);
  }
  process.exit(2);
}
NODE
  then
    echo "[remote-contracts] strict preflight failed; see ${PRECHECK_JSON}" >&2
    return 2
  fi
}

sanitize_summary_value() {
  printf '%s' "${1:-}" | tr '\t\r\n' '   '
}

tail_supports_pid_flag() {
  tail --help 2>&1 | grep -q -- '--pid'
}

stream_log_until_pid_exits() {
  local cmd_pid="$1"
  local log_file="$2"

  if tail_supports_pid_flag; then
    tail -n +1 -f --pid="${cmd_pid}" "${log_file}" || true
    return 0
  fi

  local next_line=1
  while kill -0 "${cmd_pid}" >/dev/null 2>&1; do
    if [[ -f "${log_file}" ]]; then
      local total_lines="0"
      total_lines="$(wc -l <"${log_file}" 2>/dev/null || printf '0')"
      if [[ "${total_lines}" -ge "${next_line}" ]]; then
        sed -n "${next_line},${total_lines}p" "${log_file}" || true
        next_line=$((total_lines + 1))
      fi
    fi
    sleep 1
  done

  if [[ -f "${log_file}" ]]; then
    local total_lines="0"
    total_lines="$(wc -l <"${log_file}" 2>/dev/null || printf '0')"
    if [[ "${total_lines}" -ge "${next_line}" ]]; then
      sed -n "${next_line},${total_lines}p" "${log_file}" || true
    fi
  fi
}

sweep_webkit_automation_helpers() {
  if ! command -v ps >/dev/null 2>&1; then
    return 0
  fi

  local pids=()
  local pid cmd
  while read -r pid cmd; do
    if [[ -z "${pid}" || -z "${cmd:-}" ]]; then
      continue
    fi
    case "${cmd}" in
      *WebKitWebDriver*|*wkwebdriver*|*WebKitWebProcess*|*WebKitNetworkProcess*|*WebKitGPUProcess*|*WebKitPluginProcess*|*WebKitStorageProcess*|*WebKitWebExtension*)
        pids+=("${pid}")
        ;;
    esac
  done < <(ps -Ao pid=,command= 2>/dev/null || true)

  if [[ "${#pids[@]}" -gt 0 ]]; then
    kill -9 "${pids[@]}" >/dev/null 2>&1 || true
  fi
}

write_process_snapshot() {
  local out_path="$1"
  if ! command -v ps >/dev/null 2>&1; then
    return 0
  fi
  ps -Ao pid=,ppid=,stat=,etime=,command= >"${out_path}" 2>&1 || true
}

write_launch_diagnostics() {
  local out_path="$1"
  local app_path="${CTX_DESKTOP_APP_PATH:-}"
  local app_root="${app_path%/AppRun}"
  local -a launch_paths=()
  if [[ -n "${app_path}" ]]; then
    launch_paths+=(
      "${app_path}"
      "${app_path}.wrapped"
      "${app_root}/usr/bin/ctx"
      "${app_root}/usr/bin/ctx-daemon"
      "${app_root}/apprun-hooks/linuxdeploy-plugin-gtk.sh"
    )
  fi
  {
    printf 'CTX_DESKTOP_APP_PATH=%s\n' "${app_path}"
    printf 'CTX_AUTOMATION_SHIPPED_APP_BUNDLES_DIR=%s\n' "${CTX_AUTOMATION_SHIPPED_APP_BUNDLES_DIR:-}"
    printf 'CTX_AUTOMATION_SHIPPED_APP_DAEMON_DATA_DIR=%s\n' "${CTX_AUTOMATION_SHIPPED_APP_DAEMON_DATA_DIR:-}"
    printf 'APPDIR=%s\n' "${APPDIR:-}"
    printf 'APPIMAGE=%s\n' "${APPIMAGE:-}"
    printf 'APPIMAGE_EXTRACT_AND_RUN=%s\n' "${APPIMAGE_EXTRACT_AND_RUN:-}"
    printf 'ARGV0=%s\n' "${ARGV0:-}"
    printf 'CTX_APPIMAGE_PATH=%s\n' "${CTX_APPIMAGE_PATH:-}"
    printf 'DISPLAY=%s\n' "${DISPLAY:-}"
    printf 'XDG_RUNTIME_DIR=%s\n' "${XDG_RUNTIME_DIR:-}"
    printf 'HOME=%s\n' "${HOME:-}"
    for launch_path in "${launch_paths[@]}"; do
      [[ -n "${launch_path}" ]] || continue
      if [[ -e "${launch_path}" ]]; then
        ls -l "${launch_path}" 2>&1 || true
      else
        printf 'missing %s\n' "${launch_path}"
      fi
    done
  } >"${out_path}" 2>&1 || true
}

process_elapsed_seconds() {
  local elapsed="$1"
  local days=0
  local time_part="$elapsed"
  if [[ "$time_part" == *-* ]]; then
    days="${time_part%%-*}"
    time_part="${time_part#*-}"
  fi

  local first="" second="" third=""
  IFS=: read -r first second third <<<"$time_part"
  local hours=0
  local minutes="$first"
  local seconds="$second"
  if [[ -n "${third:-}" ]]; then
    hours="$first"
    minutes="$second"
    seconds="$third"
  fi

  if ! [[ "$days" =~ ^[0-9]+$ && "$hours" =~ ^[0-9]+$ && "$minutes" =~ ^[0-9]+$ && "$seconds" =~ ^[0-9]+$ ]]; then
    return 1
  fi
  printf '%s\n' $((10#$days * 86400 + 10#$hours * 3600 + 10#$minutes * 60 + 10#$seconds))
}

command_is_ctx_remote_real_xvfb() {
  local cmd="$1"
  case "${cmd}" in
    *Xvfb*"remote-contracts"*"automation-attempt-"*"/tmp/xvfb-run."*"/Xauthority"* | \
    *Xvfb*"remote-workspace-e2e"* | \
    *Xvfb*"remote-workspace-desktop-launch-smoke"* | \
    *Xvfb*".ctx/volatile/artifacts/ctx-desktop-e2e"*) return 0 ;;
  esac
  return 1
}

sweep_stale_xvfb_processes() {
  if [[ "${CTX_REMOTE_REAL_CI_SWEEP_STALE_XVFB:-1}" != "1" ]]; then
    return 0
  fi
  local min_age_seconds="${CTX_REMOTE_REAL_CI_STALE_XVFB_MIN_AGE_SECONDS:-900}"
  if ! [[ "$min_age_seconds" =~ ^[0-9]+$ ]]; then
    echo "error: CTX_REMOTE_REAL_CI_STALE_XVFB_MIN_AGE_SECONDS must be a non-negative integer" >&2
    return 2
  fi
  if ! command -v ps >/dev/null 2>&1; then
    return 0
  fi

  local pids=()
  local pid ppid elapsed cmd age_seconds
  while read -r pid ppid elapsed cmd; do
    if [[ -z "${pid}" || -z "${ppid}" || -z "${elapsed}" || -z "${cmd:-}" ]]; then
      continue
    fi
    if [[ "$ppid" != "1" ]]; then
      continue
    fi
    if ! command_is_ctx_remote_real_xvfb "$cmd"; then
      continue
    fi
    if ! age_seconds="$(process_elapsed_seconds "$elapsed")"; then
      continue
    fi
    if [[ "$age_seconds" -lt "$min_age_seconds" ]]; then
      continue
    fi
    pids+=("${pid}")
  done < <(ps -Ao pid=,ppid=,etime=,command= 2>/dev/null || true)

  if [[ "${#pids[@]}" -gt 0 ]]; then
    kill -9 "${pids[@]}" >/dev/null 2>&1 || true
  fi
}

sweep_xvfb_processes_for_tmp_dir() {
  local tmp_dir="$1"
  if [[ -z "$tmp_dir" ]]; then
    return 0
  fi
  if ! command -v ps >/dev/null 2>&1; then
    return 0
  fi

  local resolved_tmp_dir
  resolved_tmp_dir="$(cd "$tmp_dir" 2>/dev/null && pwd -P || printf '%s' "$tmp_dir")"
  local pids=()
  local pid cmd
  while read -r pid cmd; do
    if [[ -z "${pid}" || -z "${cmd:-}" ]]; then
      continue
    fi
    case "${cmd}" in
      *Xvfb*"${tmp_dir}"* | *Xvfb*"${resolved_tmp_dir}"*) pids+=("${pid}") ;;
    esac
  done < <(ps -Ao pid=,command= 2>/dev/null || true)

  if [[ "${#pids[@]}" -gt 0 ]]; then
    kill -9 "${pids[@]}" >/dev/null 2>&1 || true
  fi
}

sweep_controller_app_processes() {
  local app_path="${CTX_DESKTOP_APP_PATH:-}"
  if [[ -z "${app_path}" ]]; then
    return 0
  fi
  local app_dir
  app_dir="$(cd "$(dirname "${app_path}")" 2>/dev/null && pwd -P || dirname "${app_path}")"
  if [[ -z "${app_dir}" || ! -d "${app_dir}" ]]; then
    return 0
  fi
  if ! command -v ps >/dev/null 2>&1; then
    return 0
  fi

  local pids=()
  local pid cmd
  while read -r pid cmd; do
    if [[ -z "${pid}" || -z "${cmd:-}" ]]; then
      continue
    fi
    case "${cmd}" in
      *"${app_dir}"*) pids+=("${pid}") ;;
    esac
  done < <(ps -Ao pid=,command= 2>/dev/null || true)

  if [[ "${#pids[@]}" -gt 0 ]]; then
    kill -9 "${pids[@]}" >/dev/null 2>&1 || true
  fi
}

sweep_local_automation_daemons() {
  if ! command -v ps >/dev/null 2>&1; then
    return 0
  fi

  local pids=()
  local pid cmd
  while read -r pid cmd; do
    if [[ -z "${pid}" || -z "${cmd:-}" ]]; then
      continue
    fi
    case "${cmd}" in
      *ctx-daemon*" serve "* | *"/ctx serve "* | *" ctx serve "*) ;;
      *) continue ;;
    esac
    case "${cmd}" in
      *"--data-dir "*"ctx-desktop-e2e-app-daemon-"* | \
      *"--data-dir="*"ctx-desktop-e2e-app-daemon-"* | \
      *"--data-dir "*"${ARTIFACT_DIR}"*"/controller-daemon-data"* | \
      *"--data-dir="*"${ARTIFACT_DIR}"*"/controller-daemon-data"* | \
      *"--data-dir "*"remote-workspace-e2e"* | \
      *"--data-dir="*"remote-workspace-e2e"*)
        pids+=("${pid}")
        ;;
    esac
  done < <(ps -Ao pid=,command= 2>/dev/null || true)

  if [[ "${#pids[@]}" -gt 0 ]]; then
    kill -9 "${pids[@]}" >/dev/null 2>&1 || true
  fi
}

sweep_local_automation_processes() {
  sweep_controller_app_processes
  sweep_webkit_automation_helpers
  sweep_local_automation_daemons
}

classify_report() {
  local report_path="$1"
  local allow_skip="$2"
  node -e '
const fs = require("node:fs");
const reportPath = process.argv[1];
const allowSkip = process.argv[2] === "1";
const report = JSON.parse(fs.readFileSync(reportPath, "utf8"));
const result = String(report.result || (report.skipped ? "skipped" : "unknown")).trim().toLowerCase();
const reason = String(report.reason || report.skip_reason || "").replace(/[\t\r\n]+/g, " ").trim();
if (result === "passed" || result === "pass") {
  process.stdout.write(`pass\t0\t${reason}\n`);
  process.exit(0);
}
if (result === "skipped") {
  const status = allowSkip ? "skipped" : "infra_unavailable";
  const code = allowSkip ? "0" : "86";
  process.stdout.write(`${status}\t${code}\t${reason || "lane skipped"}\n`);
  process.exit(0);
}
process.stdout.write(`fail\t1\t${reason || result || "report marked failed"}\n`);
' "${report_path}" "${allow_skip}"
}

is_retryable_wdio_session_start_failure() {
  local log_path="$1"
  [[ -f "${log_path}" ]] || return 1
  grep -Eq 'Failed to create a session|Could not start a new session|POST[[:space:]]+/session|/session[[:space:]]' "${log_path}" || return 1
  grep -Eq 'UND_ERR_HEADERS_TIMEOUT|hyper::Error\(IncompleteMessage\)' "${log_path}"
}

core_package_manager() {
  node - "${ROOT}/core/package.json" <<'NODE'
const fs = require("node:fs");
const packageJsonPath = process.argv[2];
const packageJson = JSON.parse(fs.readFileSync(packageJsonPath, "utf8"));
const packageManager = String(packageJson.packageManager || "").trim();
if (!packageManager) {
  console.error(`error: ${packageJsonPath} is missing packageManager`);
  process.exit(2);
}
process.stdout.write(packageManager);
NODE
}

prepare_attempt_corepack() {
  local package_manager
  package_manager="$(core_package_manager)"
  case "${package_manager}" in
    pnpm@*) ;;
    *)
      echo "error: remote CI expected core/package.json packageManager to pin pnpm, got '${package_manager}'" >&2
      return 2
      ;;
  esac
  if ! command -v corepack >/dev/null 2>&1; then
    echo "error: remote CI requires Corepack to activate ${package_manager}" >&2
    return 2
  fi
  # pnpm's Corepack shim resolves the package manager before honoring -C/--dir.
  # Activate the core workspace pin inside the isolated attempt COREPACK_HOME.
  COREPACK_ENABLE_DOWNLOAD_PROMPT=0 corepack prepare "${package_manager}" --activate
}

printf "lane\tstatus\texit_code\tartifact_dir\treason\n" >"${SUMMARY_TSV}"

run_lane() {
  local lane="$1"
  local lane_dir="$2"
  local report_path="$3"
  shift 3
  local -a cmd=("$@")
  local wdio_log="${lane_dir}/wdio.log"
  local status="pass"
  local lane_exit=0
  local reason=""
  local max_attempts="${CTX_REMOTE_REAL_CI_AUTOMATION_ATTEMPTS:-${CTX_REMOTE_WORKSPACE_E2E_AUTOMATION_ATTEMPTS:-2}}"

  if ! [[ "${max_attempts}" =~ ^[0-9]+$ ]] || [[ "${max_attempts}" -lt 1 ]]; then
    echo "error: CTX_REMOTE_REAL_CI_AUTOMATION_ATTEMPTS must be a positive integer" >&2
    return 2
  fi

  if [[ "${DRY_RUN}" == "1" ]]; then
    printf "dry-run lane=%s cmd=%s\n" "${lane}" "${cmd[*]}" >"${wdio_log}"
    status="dry-run"
    lane_exit=0
    reason="dry-run"
  else
    local attempt=1
    local cmd_exit=0
    while true; do
      local attempt_dir="${lane_dir}/automation-attempt-${attempt}"
      local attempt_log="${lane_dir}/wdio-attempt-${attempt}.log"
      local attempt_tmp_dir="${attempt_dir}/tmp"
      local attempt_daemon_data_dir="${attempt_dir}/controller-daemon-data"
      local attempt_xdg_token
      attempt_xdg_token="$(printf '%s' "${BUILDKITE_JOB_ID:-local}-${lane}-${attempt}-$$" | tr -c 'A-Za-z0-9._-' '_')"
      # WebKit helpers create short Unix sockets under XDG_RUNTIME_DIR.
      local attempt_xdg_dir="/tmp/ctx-remote-real-xdg-${attempt_xdg_token}"
      local attempt_xdg_runtime_dir="${attempt_xdg_dir}/runtime"
      local attempt_home_dir="${attempt_xdg_dir}/home"
      local attempt_corepack_home="${attempt_xdg_dir}/corepack"
      rm -f "${report_path}"
      rm -rf "${attempt_xdg_dir}"
      mkdir -p \
        "${attempt_tmp_dir}" \
        "${attempt_daemon_data_dir}" \
        "${attempt_xdg_runtime_dir}" \
        "${attempt_home_dir}" \
        "${attempt_xdg_dir}/config" \
        "${attempt_xdg_dir}/cache" \
        "${attempt_xdg_dir}/data" \
        "${attempt_corepack_home}"
      chmod 700 "${attempt_xdg_runtime_dir}"
      touch "${attempt_log}"
      write_process_snapshot "${attempt_dir}/processes-before-sweep.log"
      sweep_stale_xvfb_processes
      sweep_local_automation_processes
      write_process_snapshot "${attempt_dir}/processes-after-preflight-sweep.log"
      write_launch_diagnostics "${attempt_dir}/launch-env.txt"
      set +e
      (
        cd "${ROOT}"
        export CTX_AUTOMATION_ALLOW_PREP_APP_PROCESS_SWEEP="${CTX_AUTOMATION_ALLOW_PREP_APP_PROCESS_SWEEP:-1}"
        export CTX_AUTOMATION_ALLOW_STALE_HELPER_SWEEP="${CTX_AUTOMATION_ALLOW_STALE_HELPER_SWEEP:-1}"
        export CTX_AUTOMATION_SHIPPED_APP_DAEMON_DATA_DIR="${attempt_daemon_data_dir}"
        export CTX_AUTOMATION_TMPDIR="${attempt_tmp_dir}"
        export CTX_AUTOMATION_APP_LAUNCH_LOG="${attempt_dir}/app-launch.log"
        export CTX_AUTOMATION_CN_DRIVER_LOG="${attempt_dir}/tauri-driver.log"
        export HOME="${attempt_home_dir}"
        export TMPDIR="${attempt_tmp_dir}"
        export TMP="${attempt_tmp_dir}"
        export TEMP="${attempt_tmp_dir}"
        export XDG_RUNTIME_DIR="${attempt_xdg_runtime_dir}"
        export XDG_CONFIG_HOME="${attempt_xdg_dir}/config"
        export XDG_CACHE_HOME="${attempt_xdg_dir}/cache"
        export XDG_DATA_HOME="${attempt_xdg_dir}/data"
        export COREPACK_HOME="${attempt_corepack_home}"
        export COREPACK_ENABLE_DOWNLOAD_PROMPT=0
        prepare_attempt_corepack
        "${cmd[@]}"
      ) >"${attempt_log}" 2>&1 &
      local cmd_pid="$!"
      stream_log_until_pid_exits "${cmd_pid}" "${attempt_log}"
      wait "${cmd_pid}"
      cmd_exit="$?"
      set -e
      write_process_snapshot "${attempt_dir}/processes-after-automation.log"
      sweep_xvfb_processes_for_tmp_dir "${attempt_tmp_dir}"
      sweep_stale_xvfb_processes
      sweep_local_automation_processes
      write_process_snapshot "${attempt_dir}/processes-after-automation-sweep.log"
      rm -rf "${attempt_xdg_dir}"
      cp "${attempt_log}" "${wdio_log}"
      if [[ "${cmd_exit}" -eq 0 ]]; then
        break
      fi
      if [[ "${attempt}" -ge "${max_attempts}" ]]; then
        break
      fi
      if [[ -f "${report_path}" ]]; then
        break
      fi
      if ! is_retryable_wdio_session_start_failure "${attempt_log}"; then
        break
      fi
      echo "[remote-contracts] ${lane}: retrying startup-only WebDriver session failure after attempt ${attempt}; log: ${attempt_log}" >&2
      attempt=$((attempt + 1))
    done

    if [[ -f "${report_path}" ]]; then
      local parsed_status=""
      local parsed_exit=""
      local parsed_reason=""
      IFS=$'\t' read -r parsed_status parsed_exit parsed_reason < <(classify_report "${report_path}" "${ALLOW_SKIP}")
      if [[ "${cmd_exit}" -ne 0 ]]; then
        status="fail"
        lane_exit="${cmd_exit}"
        reason="${parsed_reason:-command exited ${cmd_exit}}"
      else
        status="${parsed_status}"
        lane_exit="${parsed_exit}"
        reason="${parsed_reason}"
      fi
    else
      if [[ "${cmd_exit}" -ne 0 ]]; then
        status="fail"
        lane_exit="${cmd_exit}"
        reason="command exited ${cmd_exit}"
      else
        status="fail"
        lane_exit=1
        reason="missing report ${report_path}"
      fi
    fi
  fi

  printf "%s\t%s\t%s\t%s\t%s\n" \
    "${lane}" \
    "${status}" \
    "${lane_exit}" \
    "${lane_dir}" \
    "$(sanitize_summary_value "${reason}")" >>"${SUMMARY_TSV}"

  if [[ -n "${reason}" ]]; then
    echo "[remote-contracts] ${lane}: ${status} (${reason})" >&2
  else
    echo "[remote-contracts] ${lane}: ${status}" >&2
  fi

  return "${lane_exit}"
}

run_preflight

host_status=0
if [[ "${RUN_HOST}" == "1" ]]; then
  host_report="${HOST_DIR}/contract-report.json"
  host_first_turn="${HOST_DIR}/first-turn.json"
  run_lane \
    "remote-host" \
    "${HOST_DIR}" \
    "${host_report}" \
    env \
    "CARGO_TARGET_DIR=${ROOT}/core/target" \
    "CTX_AUTOMATION_REMOTE_STRICT=${STRICT_REQUIRED}" \
    "CTX_AUTOMATION_REMOTE_ALLOW_SKIP=${ALLOW_SKIP}" \
    "CTX_AUTOMATION_REMOTE_AUTH_TEST_MODE=${CTX_AUTOMATION_REMOTE_AUTH_TEST_MODE:-key}" \
    "CTX_AUTOMATION_REMOTE_EXPECT_CONNECT_FAILURE=0" \
    "CTX_AUTOMATION_REMOTE_REQUIRE_FIRST_TURN_SUCCESS=${REQUIRE_FIRST_TURN_SUCCESS}" \
    "CTX_REMOTE_BOOTSTRAP_CONTRACT_REPORT=${host_report}" \
    "CTX_REMOTE_BOOTSTRAP_FIRST_TURN_REPORT=${host_first_turn}" \
    pnpm -C "${ROOT}/core/apps/desktop" test:automation:remote-bootstrap
  host_status=$?
else
  printf "%s\t%s\t%s\t%s\t%s\n" \
    "remote-host" \
    "skipped" \
    "0" \
    "${HOST_DIR}" \
    "disabled by run_host=0" >>"${SUMMARY_TSV}"
  echo "[remote-contracts] remote-host: skipped (disabled by run_host=0)" >&2
fi

container_status=0
if [[ "${RUN_CONTAINER}" == "1" ]]; then
  container_report="${CONTAINER_DIR}/contract-report.json"
  run_lane \
    "remote-container" \
    "${CONTAINER_DIR}" \
    "${container_report}" \
    env \
    "CARGO_TARGET_DIR=${ROOT}/core/target" \
    "CTX_AUTOMATION_REMOTE_STRICT=${STRICT_REQUIRED}" \
    "CTX_AUTOMATION_REMOTE_ALLOW_SKIP=${ALLOW_SKIP}" \
    "CTX_AUTOMATION_SCENARIOS=${CTX_AUTOMATION_SCENARIOS:-remote-new-sandbox}" \
    "CTX_AUTOMATION_REMOTE_REQUIRE_FIRST_TURN_SUCCESS=${REQUIRE_FIRST_TURN_SUCCESS}" \
    "CTX_REMOTE_CONTAINER_CONTRACT_REPORT=${container_report}" \
    pnpm -C "${ROOT}/core/apps/desktop" test:automation:remote-container-contract
  container_status=$?
fi

{
  echo "remote_contracts"
  echo "artifact_dir=${ARTIFACT_DIR}"
  echo "preflight=${PRECHECK_JSON}"
  echo "summary_tsv=${SUMMARY_TSV}"
  echo "host_status=${host_status}"
  echo "container_status=${container_status}"
  echo "run_host=${RUN_HOST}"
  echo "run_container=${RUN_CONTAINER}"
  echo "strict_required=${STRICT_REQUIRED}"
  echo "allow_skip=${ALLOW_SKIP}"
  echo "dry_run=${DRY_RUN}"
} >"${SUMMARY_TXT}"

echo "[remote-contracts] summary: ${SUMMARY_TXT}" >&2
echo "[remote-contracts] tsv: ${SUMMARY_TSV}" >&2

if [[ "${host_status}" -ne 0 ]]; then
  exit "${host_status}"
fi
if [[ "${RUN_CONTAINER}" == "1" && "${container_status}" -ne 0 ]]; then
  exit "${container_status}"
fi

exit 0
