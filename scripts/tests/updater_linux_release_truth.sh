#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")"/../.. && pwd)"
RUN_ID="$(date -u +%Y%m%dT%H%M%SZ)"
ARTIFACT_DIR="${CTX_UPDATER_LINUX_PROOF_ARTIFACT_DIR:-${ROOT}/core/apps/desktop/automation/artifacts/updater-linux-proof/${RUN_ID}}"
INSTALL_URL="${CTX_UPDATER_LINUX_PROOF_INSTALL_URL:-https://ctx.rs/install}"
BOOTSTRAP_CHANNEL="${CTX_UPDATER_LINUX_PROOF_BOOTSTRAP_CHANNEL:-stable}"
TARGET_CHANNEL="${CTX_UPDATER_LINUX_PROOF_TARGET_CHANNEL:-${RELEASE_STORAGE_CHANNEL:-${RELEASE_CHANNEL:-e2e}}}"
REQUIRE_VERSION_CHANGE="${CTX_UPDATER_LINUX_PROOF_REQUIRE_VERSION_CHANGE:-1}"
REPORT_PATH="${ARTIFACT_DIR}/summary.json"
INSTALL_SCRIPT="${ARTIFACT_DIR}/install.sh"
INSTALL_DOWNLOAD_LOG="${ARTIFACT_DIR}/install-download.log"
INSTALL_STDOUT="${ARTIFACT_DIR}/install.stdout.log"
INSTALL_STDERR="${ARTIFACT_DIR}/install.stderr.log"
UPDATER_REPORT="${ARTIFACT_DIR}/native-update.json"
UP_TO_DATE_REPORT="${ARTIFACT_DIR}/native-up-to-date.json"
WEBKIT_PREP_LOG="${ARTIFACT_DIR}/webkit-prep.log"
RUNTIME_INSTALL_LOG="${ARTIFACT_DIR}/runtime-install.log"
WIZARD_LOG="${ARTIFACT_DIR}/workspace-wizard.log"
APPIMAGE_AUTOMATION_TARGETS_LOG="${ARTIFACT_DIR}/appimage-automation-targets.log"
DAEMON_CLEANUP_LOG="${ARTIFACT_DIR}/daemon-cleanup.log"
EXTRACT_DIR="${ARTIFACT_DIR}/appimage-extract"
WDIO_CONNECTION_RETRY_TIMEOUT_MS="${CTX_UPDATER_LINUX_PROOF_WDIO_CONNECTION_RETRY_TIMEOUT_MS:-300000}"
home_dir=""

mkdir -p "${ARTIFACT_DIR}"

upload_artifacts_on_buildkite() {
  if [[ -z "${BUILDKITE:-}" ]] || ! command -v buildkite-agent >/dev/null 2>&1; then
    return
  fi
  buildkite-agent artifact upload "${ARTIFACT_DIR}/*.json" >/dev/null 2>&1 || true
  buildkite-agent artifact upload "${ARTIFACT_DIR}/*.log" >/dev/null 2>&1 || true
  buildkite-agent artifact upload "${ARTIFACT_DIR}/volatile/artifacts/ctx-desktop-e2e/**/*.log" >/dev/null 2>&1 || true
  buildkite-agent artifact upload "${ARTIFACT_DIR}/volatile/artifacts/ctx-desktop-e2e/**/*.png" >/dev/null 2>&1 || true
}

log_daemon_cleanup() {
  printf '%s\n' "$*" >>"${DAEMON_CLEANUP_LOG}" 2>/dev/null || true
  printf '%s\n' "$*" >&2
}

proof_daemon_command_matches() {
  local cmd="$1"
  local data_dir="$2"
  if [[ -z "${cmd}" ]]; then
    return 1
  fi
  if [[ "${cmd}" == *"${data_dir}"* ]]; then
    return 0
  fi
  [[ "${cmd}" == *"ctx-daemon"* || "${cmd}" == *"/ctx serve"* || "${cmd}" == *" ctx serve"* ]]
}

stop_proof_daemons() {
  local proof_home="${home_dir:-}"
  if [[ -z "${proof_home}" ]]; then
    return 0
  fi
  local data_dir="${proof_home}/.ctx"
  local lock_file="${data_dir}/daemon.lock"
  local pids=()

  if [[ -f "${lock_file}" ]]; then
    local lock_pid=""
    lock_pid="$(head -n 1 "${lock_file}" 2>/dev/null | tr -cd '0-9' || true)"
    if [[ "${lock_pid}" =~ ^[0-9]+$ && "${lock_pid}" != "$$" ]]; then
      local lock_cmd=""
      lock_cmd="$(ps -p "${lock_pid}" -o command= 2>/dev/null || true)"
      if proof_daemon_command_matches "${lock_cmd}" "${data_dir}"; then
        pids+=("${lock_pid}")
      elif [[ -n "${lock_cmd}" ]]; then
        log_daemon_cleanup "[updater-linux-proof] refusing to stop non-daemon lock pid=${lock_pid} cmd=${lock_cmd}"
      fi
    fi
  fi

  while IFS= read -r line; do
    local pid="${line%% *}"
    local cmd="${line#* }"
    if [[ "${pid}" =~ ^[0-9]+$ && "${pid}" != "$$" && "${cmd}" == *"${data_dir}"* ]]; then
      if proof_daemon_command_matches "${cmd}" "${data_dir}"; then
        pids+=("${pid}")
      fi
    fi
  done < <(ps -eo pid=,command= 2>/dev/null || true)

  if [[ "${#pids[@]}" -eq 0 ]]; then
    return 0
  fi

  log_daemon_cleanup "[updater-linux-proof] stopping proof daemon pids=${pids[*]} data_dir=${data_dir}"
  kill "${pids[@]}" 2>/dev/null || true
  for _ in {1..50}; do
    local alive=0
    for pid in "${pids[@]}"; do
      if kill -0 "${pid}" 2>/dev/null; then
        alive=1
        break
      fi
    done
    if [[ "${alive}" == "0" ]]; then
      return 0
    fi
    sleep 0.1
  done
  kill -9 "${pids[@]}" 2>/dev/null || true
}

cleanup_on_exit() {
  stop_proof_daemons || true
  upload_artifacts_on_buildkite
}

trap cleanup_on_exit EXIT

write_report() {
  local status="$1"
  local reason="$2"
  REPORT_PATH="${REPORT_PATH}" \
  REPORT_STATUS="${status}" \
  REPORT_REASON="${reason}" \
  REPORT_INSTALL_URL="${INSTALL_URL}" \
  REPORT_BOOTSTRAP_CHANNEL="${BOOTSTRAP_CHANNEL}" \
  REPORT_TARGET_CHANNEL="${TARGET_CHANNEL}" \
  REPORT_INSTALL_SCRIPT="${INSTALL_SCRIPT}" \
  REPORT_INSTALL_DOWNLOAD_LOG="${INSTALL_DOWNLOAD_LOG}" \
  REPORT_INSTALL_STDOUT="${INSTALL_STDOUT}" \
  REPORT_INSTALL_STDERR="${INSTALL_STDERR}" \
  REPORT_UPDATER_REPORT="${UPDATER_REPORT}" \
  REPORT_UP_TO_DATE_REPORT="${UP_TO_DATE_REPORT}" \
  REPORT_WEBKIT_PREP_LOG="${WEBKIT_PREP_LOG}" \
  REPORT_APPIMAGE_AUTOMATION_TARGETS_LOG="${APPIMAGE_AUTOMATION_TARGETS_LOG}" \
  REPORT_RUNTIME_INSTALL_LOG="${RUNTIME_INSTALL_LOG}" \
  REPORT_WIZARD_LOG="${WIZARD_LOG}" \
  REPORT_ARTIFACT_DIR="${ARTIFACT_DIR}" \
  REPORT_BEFORE_VERSION="${before_version:-}" \
  REPORT_AFTER_VERSION="${after_version:-}" \
  node <<'NODE'
const fs = require("node:fs");
const path = require("node:path");

const readJson = (filePath) => {
  try {
    if (!filePath || !fs.existsSync(filePath)) return null;
    return JSON.parse(fs.readFileSync(filePath, "utf8"));
  } catch {
    return null;
  }
};

const payload = {
  schema_version: 1,
  status: process.env.REPORT_STATUS || "unknown",
  reason: process.env.REPORT_REASON || "",
  artifact_dir: process.env.REPORT_ARTIFACT_DIR || "",
  install_url: process.env.REPORT_INSTALL_URL || "",
  bootstrap_channel: process.env.REPORT_BOOTSTRAP_CHANNEL || "",
  target_channel: process.env.REPORT_TARGET_CHANNEL || "",
  versions: {
    before: process.env.REPORT_BEFORE_VERSION || "",
    after: process.env.REPORT_AFTER_VERSION || "",
  },
  logs: {
    install_script: process.env.REPORT_INSTALL_SCRIPT || "",
    install_download: process.env.REPORT_INSTALL_DOWNLOAD_LOG || "",
    install_stdout: process.env.REPORT_INSTALL_STDOUT || "",
    install_stderr: process.env.REPORT_INSTALL_STDERR || "",
    webkit_prep: process.env.REPORT_WEBKIT_PREP_LOG || "",
    appimage_automation_targets: process.env.REPORT_APPIMAGE_AUTOMATION_TARGETS_LOG || "",
    runtime_install: process.env.REPORT_RUNTIME_INSTALL_LOG || "",
    workspace_wizard: process.env.REPORT_WIZARD_LOG || "",
  },
  reports: {
    updater: readJson(process.env.REPORT_UPDATER_REPORT || ""),
    up_to_date: readJson(process.env.REPORT_UP_TO_DATE_REPORT || ""),
  },
};

fs.mkdirSync(path.dirname(process.env.REPORT_PATH), { recursive: true });
fs.writeFileSync(process.env.REPORT_PATH, `${JSON.stringify(payload, null, 2)}\n`, "utf8");
NODE
}

matches_log() {
  local pattern="$1"
  if command -v rg >/dev/null 2>&1; then
    rg -qi "${pattern}" "${WIZARD_LOG}"
    return
  fi
  grep -Eqi "${pattern}" "${WIZARD_LOG}"
}

prepare_webkit_runtime() {
  if ! bash "${ROOT}/scripts/install_desktop_deps_linux_ubuntu.sh" --check; then
    bash "${ROOT}/scripts/install_desktop_deps_linux_ubuntu.sh" || return
  fi
  bash -lc 'source "$1"; release_prepare_webkit_browser' _ "${ROOT}/scripts/buildbuddy/release_job_lib.sh" || return
  command -v WebKitWebDriver || return
}

AUTOMATION_APP_DIR=""
AUTOMATION_APP_PATH=""

# WebKitWebDriver must launch the real Tauri binary/AppRun, while the updater
# must still target the installed AppImage path for in-place replacement.
extract_appimage_for_automation() {
  local app_path="$1"
  local label="$2"
  local target_dir="${EXTRACT_DIR}/${label}"
  local app_dir="${target_dir}/squashfs-root"
  rm -rf "${target_dir}"
  mkdir -p "${target_dir}"
  (
    cd "${target_dir}"
    "${app_path}" --appimage-extract >/dev/null
  )
  if [[ ! -d "${app_dir}" ]]; then
    echo "error: AppImage extraction did not create ${app_dir}" >&2
    return 1
  fi
  local candidate=""
  for path in "${app_dir}/AppRun" "${app_dir}/usr/bin/ctx"; do
    if [[ -x "${path}" ]]; then
      candidate="${path}"
      break
    fi
  done
  if [[ -z "${candidate}" ]]; then
    echo "error: extracted AppImage is missing an executable AppRun or usr/bin/ctx under ${app_dir}" >&2
    return 1
  fi
  AUTOMATION_APP_DIR="${app_dir}"
  AUTOMATION_APP_PATH="${candidate}"
  printf '%s\n' "${label}: appimage=${app_path} appdir=${AUTOMATION_APP_DIR} automation_app=${AUTOMATION_APP_PATH}" >>"${APPIMAGE_AUTOMATION_TARGETS_LOG}"
}

if [[ "$(uname -s)" != "Linux" ]]; then
  write_report "skipped" "linux_only"
  echo "skip: updater Linux release truth only runs on Linux"
  exit 0
fi

for cmd in curl node git timeout; do
  command -v "${cmd}" >/dev/null 2>&1 || {
    write_report "infra_unavailable" "missing_${cmd}"
    echo "error: missing required command: ${cmd}" >&2
    exit 1
  }
done

if [[ -z "${OPENROUTER_API_KEY:-}" ]]; then
  write_report "infra_unavailable" "missing_openrouter_api_key"
  echo "error: OPENROUTER_API_KEY is required for updater Linux release truth" >&2
  exit 1
fi

home_dir="${ARTIFACT_DIR}/home"
repo_dir="${ARTIFACT_DIR}/workspace"
port_base="${CTX_UPDATER_LINUX_PROOF_PORT_BASE:-$((30000 + RANDOM % 20000))}"
update_driver_port="${CTX_UPDATER_LINUX_PROOF_UPDATE_DRIVER_PORT:-${port_base}}"
update_backend_port="${CTX_UPDATER_LINUX_PROOF_UPDATE_BACKEND_PORT:-$((port_base + 1))}"
up_to_date_driver_port="${CTX_UPDATER_LINUX_PROOF_UP_TO_DATE_DRIVER_PORT:-$((port_base + 2))}"
up_to_date_backend_port="${CTX_UPDATER_LINUX_PROOF_UP_TO_DATE_BACKEND_PORT:-$((port_base + 3))}"
wizard_driver_port="${CTX_UPDATER_LINUX_PROOF_WIZARD_DRIVER_PORT:-$((port_base + 4))}"
wizard_backend_port="${CTX_UPDATER_LINUX_PROOF_WIZARD_BACKEND_PORT:-$((port_base + 5))}"
mkdir -p "${home_dir}/.local/share" "${home_dir}/.config" "${home_dir}/.cache" "${repo_dir}" "${EXTRACT_DIR}"
git init "${repo_dir}" >/dev/null 2>&1
git -C "${repo_dir}" config user.email updater-proof@example.com
git -C "${repo_dir}" config user.name UpdaterProof
printf '%s\n' 'updater linux proof' >"${repo_dir}/README.md"
git -C "${repo_dir}" add README.md >/dev/null 2>&1
git -C "${repo_dir}" commit -m init >/dev/null 2>&1

echo "[updater-linux-proof] installing bootstrap app from ${INSTALL_URL} channel=${BOOTSTRAP_CHANNEL}" >&2
if ! curl \
  -fsSL \
  --retry 3 \
  --retry-delay 2 \
  --connect-timeout 20 \
  --max-time "${CTX_UPDATER_LINUX_PROOF_INSTALL_DOWNLOAD_TIMEOUT_SECS:-180}" \
  "${INSTALL_URL}" \
  -o "${INSTALL_SCRIPT}" >"${INSTALL_DOWNLOAD_LOG}" 2>&1; then
  write_report "infra_unavailable" "installer_download_failed"
  tail -n 200 "${INSTALL_DOWNLOAD_LOG}" >&2 || true
  exit 1
fi

HOME="${home_dir}" \
XDG_DATA_HOME="${home_dir}/.local/share" \
XDG_CONFIG_HOME="${home_dir}/.config" \
XDG_CACHE_HOME="${home_dir}/.cache" \
PATH="${home_dir}/.local/bin:${PATH}" \
CTX_CHANNEL="${BOOTSTRAP_CHANNEL}" \
CTX_INSTALL_NO_OPEN=1 \
timeout "${CTX_UPDATER_LINUX_PROOF_INSTALL_TIMEOUT_SECS:-900}" sh "${INSTALL_SCRIPT}" >"${INSTALL_STDOUT}" 2>"${INSTALL_STDERR}" || {
  status=$?
  write_report "failed" "installer_failed"
  tail -n 200 "${INSTALL_STDERR}" >&2 || true
  exit "${status}"
}

echo "[updater-linux-proof] preparing linux WebKit webdriver runtime" >&2
if ! prepare_webkit_runtime >"${WEBKIT_PREP_LOG}" 2>&1; then
  write_report "infra_unavailable" "webkit_prep_failed"
  tail -n 200 "${WEBKIT_PREP_LOG}" >&2 || true
  exit 1
fi

app_path="${home_dir}/.local/share/ctx/ctx.AppImage"
launcher_path="${home_dir}/.local/bin/ctx-desktop"
for required in "${app_path}" "${launcher_path}"; do
  if [[ ! -e "${required}" ]]; then
    write_report "failed" "installer_missing_expected_artifact"
    echo "error: install flow did not create expected artifact: ${required}" >&2
    exit 1
  fi
done
chmod +x "${app_path}"
if ! extract_appimage_for_automation "${app_path}" "bootstrap"; then
  write_report "failed" "bootstrap_appimage_automation_extract_failed"
  exit 1
fi
bootstrap_automation_app_dir="${AUTOMATION_APP_DIR}"
bootstrap_automation_app_path="${AUTOMATION_APP_PATH}"

echo "[updater-linux-proof] proving auto/manual update path to channel=${TARGET_CHANNEL}" >&2
if ! HOME="${home_dir}" \
  XDG_DATA_HOME="${home_dir}/.local/share" \
  XDG_CONFIG_HOME="${home_dir}/.config" \
  XDG_CACHE_HOME="${home_dir}/.cache" \
  PATH="${home_dir}/.local/bin:${PATH}" \
  APPIMAGE_EXTRACT_AND_RUN="${APPIMAGE_EXTRACT_AND_RUN:-1}" \
  APPIMAGE="${app_path}" \
  APPDIR="${bootstrap_automation_app_dir}" \
  ARGV0="${app_path}" \
  CTX_APPIMAGE_PATH="${app_path}" \
  CTX_VOLATILE_ROOT="${ARTIFACT_DIR}/volatile" \
  CTX_AUTOMATION_SKIP_DESKTOP_PREP_RELEASE=1 \
  CTX_AUTOMATION_SKIP_APP_BUILD=1 \
  CTX_AUTOMATION_WDIO_LOG_LEVEL="${CTX_AUTOMATION_WDIO_LOG_LEVEL:-warn}" \
  CTX_AUTOMATION_CONNECTION_RETRY_TIMEOUT_MS="${CTX_AUTOMATION_CONNECTION_RETRY_TIMEOUT_MS:-${WDIO_CONNECTION_RETRY_TIMEOUT_MS}}" \
  CTX_AUTOMATION_KEEP_TMPDIR=1 \
  CTX_AUTOMATION_SHIPPED_APP=1 \
  CTX_DESKTOP_APP_PATH="${bootstrap_automation_app_path}" \
  TAURI_DRIVER_PORT="${update_driver_port}" \
  TAURI_TEST_BACKEND_PORT="${update_backend_port}" \
  CTX_UPDATER_E2E_CHANNEL="${TARGET_CHANNEL}" \
  CTX_UPDATER_NATIVE_SMOKE_REPORT="${UPDATER_REPORT}" \
  CTX_UPDATER_PROOF_EXPECT_AUTO_READY=1 \
  CTX_UPDATER_PROOF_EXPECT_UPDATE_AVAILABLE=1 \
  CTX_UPDATER_PROOF_APPLY_UPDATE=1 \
  pnpm -C "${ROOT}/core/apps/desktop" test:automation:updater-native-smoke; then
  write_report "failed" "native_update_smoke_failed"
  exit 1
fi
stop_proof_daemons

before_version="$(jq -r '.initial.summary.current_version // empty' "${UPDATER_REPORT}" 2>/dev/null || true)"
if [[ -z "${before_version}" ]]; then
  write_report "failed" "missing_bootstrap_version"
  echo "error: updater report did not capture bootstrap version" >&2
  exit 1
fi
echo "[updater-linux-proof] bootstrap app version=${before_version}" >&2

if ! extract_appimage_for_automation "${app_path}" "updated-up-to-date"; then
  write_report "failed" "updated_appimage_automation_extract_failed"
  exit 1
fi
updated_automation_app_dir="${AUTOMATION_APP_DIR}"
updated_automation_app_path="${AUTOMATION_APP_PATH}"

echo "[updater-linux-proof] proving up-to-date manual check on updated app" >&2
if ! HOME="${home_dir}" \
  XDG_DATA_HOME="${home_dir}/.local/share" \
  XDG_CONFIG_HOME="${home_dir}/.config" \
  XDG_CACHE_HOME="${home_dir}/.cache" \
  PATH="${home_dir}/.local/bin:${PATH}" \
  APPIMAGE_EXTRACT_AND_RUN="${APPIMAGE_EXTRACT_AND_RUN:-1}" \
  APPIMAGE="${app_path}" \
  APPDIR="${updated_automation_app_dir}" \
  ARGV0="${app_path}" \
  CTX_APPIMAGE_PATH="${app_path}" \
  CTX_VOLATILE_ROOT="${ARTIFACT_DIR}/volatile" \
  CTX_AUTOMATION_SKIP_DESKTOP_PREP_RELEASE=1 \
  CTX_AUTOMATION_SKIP_APP_BUILD=1 \
  CTX_AUTOMATION_WDIO_LOG_LEVEL="${CTX_AUTOMATION_WDIO_LOG_LEVEL:-warn}" \
  CTX_AUTOMATION_CONNECTION_RETRY_TIMEOUT_MS="${CTX_AUTOMATION_CONNECTION_RETRY_TIMEOUT_MS:-${WDIO_CONNECTION_RETRY_TIMEOUT_MS}}" \
  CTX_AUTOMATION_KEEP_TMPDIR=1 \
  CTX_AUTOMATION_SHIPPED_APP=1 \
  CTX_DESKTOP_APP_PATH="${updated_automation_app_path}" \
  TAURI_DRIVER_PORT="${up_to_date_driver_port}" \
  TAURI_TEST_BACKEND_PORT="${up_to_date_backend_port}" \
  CTX_UPDATER_E2E_CHANNEL="${TARGET_CHANNEL}" \
  CTX_UPDATER_NATIVE_SMOKE_REPORT="${UP_TO_DATE_REPORT}" \
  CTX_UPDATER_PROOF_EXPECT_UP_TO_DATE=1 \
  pnpm -C "${ROOT}/core/apps/desktop" test:automation:updater-native-smoke; then
  write_report "failed" "native_up_to_date_smoke_failed"
  exit 1
fi
stop_proof_daemons

after_version="$(jq -r '.summary.current_version // empty' "${UP_TO_DATE_REPORT}" 2>/dev/null || true)"
if [[ -z "${after_version}" ]]; then
  write_report "failed" "missing_updated_version"
  echo "error: updater report did not capture updated version" >&2
  exit 1
fi
echo "[updater-linux-proof] updated app version=${after_version}" >&2

if [[ "${REQUIRE_VERSION_CHANGE}" == "1" && "${before_version}" == "${after_version}" ]]; then
  write_report "failed" "version_did_not_change"
  echo "error: expected app version to change after update (before=${before_version} after=${after_version})" >&2
  exit 1
fi

rm -rf "${EXTRACT_DIR}/updated-bundle"
(
  mkdir -p "${EXTRACT_DIR}/updated-bundle"
  cd "${EXTRACT_DIR}/updated-bundle"
  "${app_path}" --appimage-extract >/dev/null
)

bundle_manifest="$(find "${EXTRACT_DIR}/updated-bundle/squashfs-root" -type f -path '*/bundles/manifest.json' -print -quit)"
if [[ -z "${bundle_manifest}" ]]; then
  write_report "failed" "updated_appimage_bundle_manifest_missing"
  echo "error: failed to locate bundled manifest.json inside updated AppImage" >&2
  exit 1
fi
bundle_dir="$(dirname "${bundle_manifest}")"

daemon_bin="$(find "${EXTRACT_DIR}/updated-bundle/squashfs-root" -type f \( -path '*/usr/bin/ctx-daemon' -o -name 'ctx-daemon-linux-x86_64' -o -name 'ctx-daemon-linux-aarch64' \) -print -quit)"
if [[ -z "${daemon_bin}" ]]; then
  write_report "failed" "updated_appimage_daemon_missing"
  echo "error: failed to locate daemon binary inside updated AppImage" >&2
  exit 1
fi
chmod +x "${daemon_bin}"

echo "[updater-linux-proof] proving provider/runtime installs from updated daemon bundles" >&2
if ! "${ROOT}/scripts/release_runtime_install_smoke.sh" \
  --daemon-bin "${daemon_bin}" \
  --bundle-dir "${bundle_dir}" \
  --provider "${CTX_UPDATER_RUNTIME_PROVIDER:-qwen}" \
  --complete >"${RUNTIME_INSTALL_LOG}" 2>&1; then
  write_report "failed" "runtime_install_smoke_failed"
  tail -n 200 "${RUNTIME_INSTALL_LOG}" >&2 || true
  exit 1
fi

echo "[updater-linux-proof] proving updated app still launches a real local workspace flow" >&2
stop_proof_daemons
set +e
HOME="${home_dir}" \
XDG_DATA_HOME="${home_dir}/.local/share" \
XDG_CONFIG_HOME="${home_dir}/.config" \
XDG_CACHE_HOME="${home_dir}/.cache" \
PATH="${home_dir}/.local/bin:${PATH}" \
APPIMAGE_EXTRACT_AND_RUN="${APPIMAGE_EXTRACT_AND_RUN:-1}" \
APPIMAGE="${app_path}" \
APPDIR="${updated_automation_app_dir}" \
ARGV0="${app_path}" \
CTX_APPIMAGE_PATH="${app_path}" \
CTX_AUTOMATION_SHIPPED_APP=1 \
CTX_AUTOMATION_SHIPPED_APP_BUNDLES_DIR="${bundle_dir}" \
CTX_AUTOMATION_SKIP_APP_BUILD=1 \
CTX_AUTOMATION_SKIP_DESKTOP_PREP_RELEASE=1 \
CTX_AUTOMATION_SCENARIOS="${CTX_AUTOMATION_SCENARIOS:-local-codex-smoke}" \
CTX_AUTOMATION_WDIO_LOG_LEVEL="${CTX_AUTOMATION_WDIO_LOG_LEVEL:-warn}" \
CTX_AUTOMATION_CONNECTION_RETRY_TIMEOUT_MS="${CTX_AUTOMATION_CONNECTION_RETRY_TIMEOUT_MS:-${WDIO_CONNECTION_RETRY_TIMEOUT_MS}}" \
CTX_AUTOMATION_KEEP_TMPDIR=1 \
CTX_VOLATILE_ROOT="${ARTIFACT_DIR}/volatile" \
CTX_DESKTOP_APP_PATH="${updated_automation_app_path}" \
TAURI_DRIVER_PORT="${wizard_driver_port}" \
TAURI_TEST_BACKEND_PORT="${wizard_backend_port}" \
"${ROOT}/scripts/desktop_smoke_with_infisical.sh" -- --spec automation/specs/workspace-wizard.spec.cjs >"${WIZARD_LOG}" 2>&1
wizard_status=$?
set -e

if [[ "${wizard_status}" -ne 0 ]]; then
  failure_reason="workspace_wizard_failed"
  if [[ -f "${WIZARD_LOG}" ]] && matches_log "local sandbox runtime is unavailable|install nerdctl|CTX_HARNESS_SANDBOX_CLI_PATH"; then
    failure_reason="runtime_bootstrap_missing"
  fi
  write_report "failed" "${failure_reason}"
  tail -n 200 "${WIZARD_LOG}" >&2 || true
  exit "${wizard_status}"
fi

write_report "passed" "linux_updater_truth_passed"
echo "[updater-linux-proof] passed; report=${REPORT_PATH}" >&2
