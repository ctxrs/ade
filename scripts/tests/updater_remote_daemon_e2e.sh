#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")"/../.. && pwd)"
source "${ROOT}/scripts/lib/updater_e2e_drill_full_paths.sh"

if [[ -z "${CTX_UPDATER_E2E_REMOTE_HOST:-${CTX_AUTOMATION_REMOTE_HOST:-}}" ]]; then
  exec "${ROOT}/scripts/updater_e2e_remote_matrix.sh" run -- bash "${ROOT}/scripts/tests/updater_remote_daemon_e2e.sh"
fi
if [[ -z "${CTX_UPDATER_E2E_SSH_KEY_PATH:-${CTX_AUTOMATION_REMOTE_SSH_KEY_PATH:-}}" ]]; then
  echo "error: CTX_UPDATER_E2E_SSH_KEY_PATH/CTX_AUTOMATION_REMOTE_SSH_KEY_PATH is required" >&2
  exit 2
fi

artifact_dir="${CTX_REMOTE_CI_ARTIFACT_DIR:-${ROOT}/core/apps/desktop/automation/artifacts/updater-remote-proof}"
mkdir -p "${artifact_dir}"
STRICT_PUBLISHED_ARTIFACTS="${CTX_UPDATER_REMOTE_E2E_STRICT_PUBLISHED_ARTIFACTS:-${CTX_UPDATER_E2E_STRICT_PUBLISHED_ARTIFACTS:-0}}"
PORT_BASE="${CTX_UPDATER_REMOTE_E2E_PORT_BASE:-$((34000 + RANDOM % 20000))}"
TAURI_DRIVER_PORT_VALUE="${CTX_UPDATER_REMOTE_E2E_DRIVER_PORT:-${TAURI_DRIVER_PORT:-${PORT_BASE}}}"
TAURI_TEST_BACKEND_PORT_VALUE="${CTX_UPDATER_REMOTE_E2E_BACKEND_PORT:-${TAURI_TEST_BACKEND_PORT:-$((PORT_BASE + 1))}}"
WDIO_CONNECTION_RETRY_TIMEOUT_MS="${CTX_UPDATER_REMOTE_E2E_WDIO_CONNECTION_RETRY_TIMEOUT_MS:-300000}"
RESOLVED_CONTROLLER_AUTOMATION_APP_PATH=""
ORIGINAL_HOME="${HOME:?set HOME}"

resolve_download_base_url() {
  if [[ -n "${CTX_UPDATER_E2E_DOWNLOAD_BASE_URL:-}" ]]; then
    printf '%s\n' "${CTX_UPDATER_E2E_DOWNLOAD_BASE_URL%/}"
    return 0
  fi
  if [[ -n "${SUPABASE_FUNCTIONS_URL:-}" ]]; then
    printf '%s\n' "${SUPABASE_FUNCTIONS_URL%/}"
    return 0
  fi
  if [[ -n "${SUPABASE_URL:-}" ]]; then
    printf '%s/functions/v1\n' "${SUPABASE_URL%/}"
    return 0
  fi
  printf '%s\n' "https://api.ctx.rs/functions/v1"
}

resolve_target_channel() {
  printf '%s\n' "${CTX_UPDATER_E2E_RELEASE_CHANNEL:-${RELEASE_STORAGE_CHANNEL:-${RELEASE_CHANNEL:-stable}}}"
}

download_published_controller_app() {
  local base_url="$1"
  local channel="$2"
  local safe_channel
  safe_channel="$(printf '%s' "$channel" | tr -c 'A-Za-z0-9._-' '_')"
  local manifest_path="${artifact_dir}/controller-${safe_channel}-latest.json"
  local app_path="${artifact_dir}/controller-${safe_channel}.AppImage"
  local meta_path="${artifact_dir}/controller-${safe_channel}-appimage.json"

  curl --fail --show-error --location --retry 3 --retry-delay 2 \
    "${base_url}/releases/${channel}/latest.json" >"${manifest_path}"
  node - <<'NODE' "${manifest_path}" "${base_url}" >"${meta_path}"
const fs = require("node:fs");
const manifestPath = process.argv[2];
const base = String(process.argv[3] || "").replace(/\/+$/u, "");
const manifest = JSON.parse(fs.readFileSync(manifestPath, "utf8"));
const platform = manifest?.platforms?.["linux-x64"];
if (!platform?.appimage?.url_path || !platform?.appimage?.sha256) {
  throw new Error("latest manifest missing linux-x64 AppImage artifact");
}
process.stdout.write(`${JSON.stringify({
  url: `${base}${platform.appimage.url_path}`,
  sha256: String(platform.appimage.sha256),
})}\n`);
NODE
  local artifact_url artifact_sha tmp_path
  artifact_url="$(node -e 'const fs=require("node:fs");const v=JSON.parse(fs.readFileSync(process.argv[1],"utf8"));process.stdout.write(String(v.url||""));' "${meta_path}")"
  artifact_sha="$(node -e 'const fs=require("node:fs");const v=JSON.parse(fs.readFileSync(process.argv[1],"utf8"));process.stdout.write(String(v.sha256||""));' "${meta_path}")"
  if [[ -z "$artifact_url" || -z "$artifact_sha" ]]; then
    echo "error: failed to resolve controller AppImage metadata for channel ${channel}" >&2
    return 1
  fi
  tmp_path="${app_path}.partial"
  rm -f "$tmp_path"
  curl --fail --show-error --location --retry 3 --retry-delay 2 --connect-timeout 20 --max-time 600 \
    -o "$tmp_path" "$artifact_url"
  printf '%s  %s\n' "$artifact_sha" "$tmp_path" | sha256sum -c - >/dev/null
  mv -f "$tmp_path" "$app_path"
  chmod +x "$app_path"
  printf '%s\n' "$app_path"
}

resolve_controller_app_for_automation() {
  local app_path="$1"
  if [[ "$(uname -s)" != "Linux" || "${app_path}" != *.AppImage ]]; then
    RESOLVED_CONTROLLER_AUTOMATION_APP_PATH="${app_path}"
    return 0
  fi

  local extract_dir="${artifact_dir}/controller-appimage-extract"
  local app_dir="${extract_dir}/squashfs-root"
  local app_run="${app_dir}/AppRun"
  rm -rf "${extract_dir}"
  mkdir -p "${extract_dir}"
  (
    cd "${extract_dir}"
    "${app_path}" --appimage-extract >/dev/null
  )
  if [[ ! -x "${app_run}" ]]; then
    echo "error: downloaded controller AppImage did not extract an executable AppRun at ${app_run}" >&2
    return 1
  fi

  local bundle_manifest=""
  bundle_manifest="$(find "${app_dir}" -type f -path '*/bundles/manifest.json' -print -quit)"
  if [[ -z "${bundle_manifest}" ]]; then
    echo "error: downloaded controller AppImage is missing bundled manifest.json under ${app_dir}" >&2
    return 1
  fi

  export APPIMAGE_EXTRACT_AND_RUN="${APPIMAGE_EXTRACT_AND_RUN:-1}"
  export APPIMAGE="${app_path}"
  export APPDIR="${app_dir}"
  export ARGV0="${app_path}"
  export CTX_APPIMAGE_PATH="${app_path}"
  export CTX_AUTOMATION_SHIPPED_APP_BUNDLES_DIR="$(dirname "${bundle_manifest}")"
  RESOLVED_CONTROLLER_AUTOMATION_APP_PATH="${app_run}"
}

write_process_snapshot() {
  local out_path="$1"
  if ! command -v ps >/dev/null 2>&1; then
    return 0
  fi
  ps -Ao pid=,ppid=,stat=,etime=,command= >"${out_path}" 2>&1 || true
}

sweep_controller_app_processes() {
  local app_path="${RESOLVED_CONTROLLER_AUTOMATION_APP_PATH:-}"
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

command_is_ctx_automation_xvfb() {
  local cmd="$1"
  case "${cmd}" in
    *Xvfb*/ctx-nightly/.artifacts/buildkite/ctx-nightly/*/updater-proof/automation-attempt-*/tmp/xvfb-run.*/Xauthority* | \
    *Xvfb*/core/apps/desktop/automation/artifacts/updater-remote-proof/automation-attempt-*/tmp/xvfb-run.*/Xauthority* | \
    *Xvfb*/core/apps/desktop/automation/artifacts/updater-linux-proof/*/volatile/artifacts/ctx-desktop-e2e/*/xvfb-run.*/Xauthority* | \
    *Xvfb*/.ctx/volatile/artifacts/ctx-desktop-e2e/*/xvfb-run.*/Xauthority*) return 0 ;;
  esac
  return 1
}

sweep_stale_xvfb_processes() {
  if [[ "${CTX_UPDATER_REMOTE_E2E_SWEEP_STALE_XVFB:-1}" != "1" ]]; then
    return 0
  fi
  local min_age_seconds="${CTX_UPDATER_REMOTE_E2E_STALE_XVFB_MIN_AGE_SECONDS:-900}"
  if ! [[ "$min_age_seconds" =~ ^[0-9]+$ ]]; then
    echo "error: CTX_UPDATER_REMOTE_E2E_STALE_XVFB_MIN_AGE_SECONDS must be a non-negative integer" >&2
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
    if ! command_is_ctx_automation_xvfb "$cmd"; then
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

if [[ -n "${CTX_DESKTOP_APP_PATH:-}" && "$STRICT_PUBLISHED_ARTIFACTS" == "1" ]]; then
  echo "error: strict published-artifact remote proof forbids CTX_DESKTOP_APP_PATH/local AppDir input" >&2
  exit 2
fi

if [[ -z "${CTX_DESKTOP_APP_PATH:-}" ]]; then
  if [[ "$STRICT_PUBLISHED_ARTIFACTS" == "1" ]]; then
    download_base_url="$(resolve_download_base_url)"
    target_channel="$(resolve_target_channel)"
    echo "[updater-remote-proof] strict published-artifact mode; downloading controller AppImage for ${target_channel}" >&2
    resolved_app_path="$(download_published_controller_app "$download_base_url" "$target_channel")"
  elif ! resolved_app_path="$(resolve_local_smoke_app_path "${ROOT}")"; then
    download_base_url="$(resolve_download_base_url)"
    target_channel="$(resolve_target_channel)"
    echo "[updater-remote-proof] local AppDir unavailable; downloading published controller AppImage for ${target_channel}" >&2
    resolved_app_path="$(download_published_controller_app "$download_base_url" "$target_channel")"
  fi
  export CTX_DESKTOP_APP_PATH="${resolved_app_path}"
fi
resolve_controller_app_for_automation "${CTX_DESKTOP_APP_PATH}"
export CTX_DESKTOP_APP_PATH="${RESOLVED_CONTROLLER_AUTOMATION_APP_PATH}"
normalize_local_smoke_app_permissions "${CTX_DESKTOP_APP_PATH}"

export CTX_UPDATER_REMOTE_E2E_REPORT="${CTX_UPDATER_REMOTE_E2E_REPORT:-${artifact_dir}/remote-proof.json}"
export CTX_UPDATER_REMOTE_PROOF_IDLE="${CTX_UPDATER_REMOTE_PROOF_IDLE:-1}"
export CTX_UPDATER_REMOTE_PROOF_PENDING_IDLE="${CTX_UPDATER_REMOTE_PROOF_PENDING_IDLE:-${CTX_UPDATER_REMOTE_PROOF_REQUIRE_COMPATIBLE:-0}}"
export CTX_UPDATER_REMOTE_PROOF_PENDING_RESTART_NOW="${CTX_UPDATER_REMOTE_PROOF_PENDING_RESTART_NOW:-${CTX_UPDATER_REMOTE_PROOF_REQUIRE_COMPATIBLE:-0}}"
export CTX_UPDATER_REMOTE_PROOF_INCOMPATIBLE_RECONNECT="${CTX_UPDATER_REMOTE_PROOF_INCOMPATIBLE_RECONNECT:-1}"
export CTX_UPDATER_REMOTE_PROOF_NO_CLIENT_AUTO="${CTX_UPDATER_REMOTE_PROOF_NO_CLIENT_AUTO:-1}"
export CTX_UPDATER_E2E_EXPECT_VERSION_CHANGE="${CTX_UPDATER_E2E_EXPECT_VERSION_CHANGE:-1}"
export CTX_AUTOMATION_SKIP_DESKTOP_PREP_RELEASE="${CTX_AUTOMATION_SKIP_DESKTOP_PREP_RELEASE:-1}"
export CTX_AUTOMATION_SKIP_APP_BUILD="${CTX_AUTOMATION_SKIP_APP_BUILD:-1}"
export CTX_AUTOMATION_SHIPPED_APP="${CTX_AUTOMATION_SHIPPED_APP:-1}"
export CTX_AUTOMATION_SKIP_REMOTE_CTX_PROVISION="${CTX_AUTOMATION_SKIP_REMOTE_CTX_PROVISION:-1}"
export CTX_AUTOMATION_ALLOW_PREP_APP_PROCESS_SWEEP="${CTX_AUTOMATION_ALLOW_PREP_APP_PROCESS_SWEEP:-1}"
export CTX_AUTOMATION_ALLOW_STALE_HELPER_SWEEP="${CTX_AUTOMATION_ALLOW_STALE_HELPER_SWEEP:-1}"
export CTX_AUTOMATION_CONNECTION_RETRY_TIMEOUT_MS="${CTX_AUTOMATION_CONNECTION_RETRY_TIMEOUT_MS:-${WDIO_CONNECTION_RETRY_TIMEOUT_MS}}"

is_retryable_wdio_session_start_failure() {
  local log_path="$1"
  if [[ ! -f "$log_path" ]]; then
    return 1
  fi
  grep -Eq 'Failed to create a session|/session' "$log_path" || return 1
  grep -Eq 'UND_ERR_HEADERS_TIMEOUT|WebDriverError.*(/session|Failed to create a session)|Request failed.*/session|Failed to create a session' "$log_path"
}

run_updater_remote_automation() {
  local max_attempts="${CTX_UPDATER_REMOTE_E2E_AUTOMATION_ATTEMPTS:-2}"
  if ! [[ "$max_attempts" =~ ^[0-9]+$ ]] || [[ "$max_attempts" -lt 1 ]]; then
    echo "error: CTX_UPDATER_REMOTE_E2E_AUTOMATION_ATTEMPTS must be a positive integer" >&2
    return 2
  fi

  local attempt=1
  local status=0
  while [[ "$attempt" -le "$max_attempts" ]]; do
    local attempt_dir="${artifact_dir}/automation-attempt-${attempt}"
    local attempt_log="${attempt_dir}/updater-remote-automation.log"
    local attempt_tmp_dir="${attempt_dir}/tmp"
    local attempt_xdg_token
    attempt_xdg_token="$(printf '%s' "${BUILDKITE_JOB_ID:-local}-${attempt}-$$" | tr -c 'A-Za-z0-9._-' '_')"
    # WebKit helpers place Unix sockets under XDG_RUNTIME_DIR, so keep this path short.
    local attempt_xdg_dir="/tmp/ctx-updater-remote-xdg-${attempt_xdg_token}"
    local attempt_xdg_runtime_dir="${attempt_xdg_dir}/runtime"
    local attempt_home_dir="${attempt_xdg_dir}/home"
    local attempt_corepack_home="${COREPACK_HOME:-${ORIGINAL_HOME}/.cache/node/corepack}"
    local attempt_driver_port=$((TAURI_DRIVER_PORT_VALUE + (attempt - 1) * 2))
    local attempt_backend_port=$((TAURI_TEST_BACKEND_PORT_VALUE + (attempt - 1) * 2))

    export CTX_AUTOMATION_TMPDIR="${attempt_tmp_dir}"
    export CTX_AUTOMATION_CN_BACKEND_LOG="${attempt_dir}/crabnebula-backend.log"
    export CTX_AUTOMATION_CN_DRIVER_LOG="${attempt_dir}/tauri-driver.log"
    export CTX_AUTOMATION_SHIPPED_APP_DAEMON_DATA_DIR="${attempt_dir}/controller-daemon-data"
    export HOME="${attempt_home_dir}"
    export XDG_RUNTIME_DIR="${attempt_xdg_runtime_dir}"
    export XDG_CONFIG_HOME="${attempt_xdg_dir}/config"
    export XDG_CACHE_HOME="${attempt_xdg_dir}/cache"
    export XDG_DATA_HOME="${attempt_xdg_dir}/data"
    export COREPACK_HOME="${attempt_corepack_home}"
    export TAURI_DRIVER_PORT="${attempt_driver_port}"
    export TAURI_TEST_BACKEND_PORT="${attempt_backend_port}"
    unset CTX_BUNDLE_DIR
    rm -rf "${attempt_xdg_dir}"
    mkdir -p \
      "${attempt_tmp_dir}" \
      "${attempt_dir}/controller-daemon-data" \
      "${HOME}" \
      "${XDG_RUNTIME_DIR}" \
      "${XDG_CONFIG_HOME}" \
      "${XDG_CACHE_HOME}" \
      "${XDG_DATA_HOME}"
    chmod 700 "${XDG_RUNTIME_DIR}"
    write_process_snapshot "${attempt_dir}/processes-before-sweep.log"
    sweep_stale_xvfb_processes
    sweep_controller_app_processes
    write_process_snapshot "${attempt_dir}/processes-after-preflight-sweep.log"

    echo "[updater-remote-proof] automation attempt ${attempt}/${max_attempts} using driver port ${TAURI_DRIVER_PORT} and backend port ${TAURI_TEST_BACKEND_PORT}" >&2
    set +e
    pnpm -C "${ROOT}/core/apps/desktop" test:automation:updater-remote 2>&1 | tee "$attempt_log"
    status="${PIPESTATUS[0]}"
    set -e
    write_process_snapshot "${attempt_dir}/processes-after-automation.log"
    sweep_xvfb_processes_for_tmp_dir "${attempt_tmp_dir}"
    sweep_stale_xvfb_processes
    sweep_controller_app_processes
    write_process_snapshot "${attempt_dir}/processes-after-automation-sweep.log"

    if [[ "$status" -eq 0 ]]; then
      rm -rf "${attempt_xdg_dir}"
      return 0
    fi
    if [[ "$attempt" -lt "$max_attempts" ]] && is_retryable_wdio_session_start_failure "$attempt_log"; then
      rm -rf "${attempt_xdg_dir}"
      echo "[updater-remote-proof] retrying startup-only WebDriver session failure after attempt ${attempt}; log: ${attempt_log}" >&2
      attempt=$((attempt + 1))
      continue
    fi
    rm -rf "${attempt_xdg_dir}"
    return "$status"
  done
  return "$status"
}

run_updater_remote_automation
