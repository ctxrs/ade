#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")"/../.. && pwd)"
RUN_ID="$(date -u +%Y%m%dT%H%M%SZ)"
ARTIFACT_DIR="${CTX_LINUX_RELEASE_TRUTH_ARTIFACT_DIR:-${ROOT}/core/apps/desktop/automation/artifacts/linux-local-install-truth/${RUN_ID}}"
INSTALL_URL="${CTX_LINUX_RELEASE_TRUTH_INSTALL_URL:-https://ctx.rs/install}"
ALLOW_PREINSTALLED_RUNTIME="${CTX_LINUX_RELEASE_TRUTH_ALLOW_PREINSTALLED_RUNTIME:-0}"
REPORT_PATH="${ARTIFACT_DIR}/report.json"
INSTALL_STDOUT="${ARTIFACT_DIR}/install.stdout.log"
INSTALL_STDERR="${ARTIFACT_DIR}/install.stderr.log"
WDIO_LOG="${ARTIFACT_DIR}/workspace-wizard.log"

mkdir -p "${ARTIFACT_DIR}"

app_path=""
bundle_dir=""
runtime_cli_path=""
runtime_wrapper_present="0"
containerd_active="0"

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "error: missing required command: $1" >&2
    exit 1
  }
}

write_report() {
  local status="$1"
  local reason="$2"
  REPORT_PATH="${REPORT_PATH}" \
  REPORT_STATUS="${status}" \
  REPORT_REASON="${reason}" \
  REPORT_INSTALL_URL="${INSTALL_URL}" \
  REPORT_APP_PATH="${app_path}" \
  REPORT_BUNDLE_DIR="${bundle_dir}" \
  REPORT_RUNTIME_CLI_PATH="${runtime_cli_path}" \
  REPORT_RUNTIME_WRAPPER_PRESENT="${runtime_wrapper_present}" \
  REPORT_CONTAINERD_ACTIVE="${containerd_active}" \
  REPORT_INSTALL_STDOUT="${INSTALL_STDOUT}" \
  REPORT_INSTALL_STDERR="${INSTALL_STDERR}" \
  REPORT_WDIO_LOG="${WDIO_LOG}" \
  node <<'NODE'
const fs = require("node:fs");

const payload = {
  schema_version: 1,
  status: process.env.REPORT_STATUS || "unknown",
  reason: process.env.REPORT_REASON || "",
  install_url: process.env.REPORT_INSTALL_URL || "",
  app_path: process.env.REPORT_APP_PATH || "",
  bundle_dir: process.env.REPORT_BUNDLE_DIR || "",
  runtime_guardrail: {
    nerdctl_path: process.env.REPORT_RUNTIME_CLI_PATH || "",
    rootful_wrapper_present: process.env.REPORT_RUNTIME_WRAPPER_PRESENT === "1",
    containerd_active: process.env.REPORT_CONTAINERD_ACTIVE === "1",
  },
  logs: {
    install_stdout: process.env.REPORT_INSTALL_STDOUT || "",
    install_stderr: process.env.REPORT_INSTALL_STDERR || "",
    wdio: process.env.REPORT_WDIO_LOG || "",
  },
};

fs.mkdirSync(require("node:path").dirname(process.env.REPORT_PATH), { recursive: true });
fs.writeFileSync(process.env.REPORT_PATH, `${JSON.stringify(payload, null, 2)}\n`, "utf8");
NODE
}

matches_log() {
  local pattern="$1"
  if command -v rg >/dev/null 2>&1; then
    rg -qi "${pattern}" "${WDIO_LOG}"
    return
  fi
  grep -Eqi "${pattern}" "${WDIO_LOG}"
}

if [[ "$(uname -s)" != "Linux" ]]; then
  write_report "skipped" "linux_only"
  echo "skip: linux local install release-truth lane only runs on Linux"
  exit 0
fi

need_cmd curl
need_cmd node
need_cmd find

if [[ -z "${OPENROUTER_API_KEY:-}" ]]; then
  write_report "infra_unavailable" "missing_openrouter_api_key"
  echo "error: OPENROUTER_API_KEY is required for the Linux local install release-truth lane" >&2
  exit 1
fi

runtime_cli_path="$(command -v nerdctl || true)"
if [[ -x /usr/local/bin/ctx-rootful-nerdctl ]]; then
  runtime_wrapper_present="1"
fi
if command -v systemctl >/dev/null 2>&1 && systemctl is-active --quiet containerd.service; then
  containerd_active="1"
fi

if [[ "${ALLOW_PREINSTALLED_RUNTIME}" != "1" ]]; then
  if [[ -n "${runtime_cli_path}" || "${runtime_wrapper_present}" == "1" || "${containerd_active}" == "1" || -n "${CTX_HARNESS_SANDBOX_CLI_PATH:-}" ]]; then
    write_report "contaminated_runner" "native_sandbox_runtime_already_present"
    echo "error: refusing to run Linux local install release-truth on a runner that already has sandbox runtime state" >&2
    exit 1
  fi
fi

home_dir="${ARTIFACT_DIR}/home"
extract_dir="${ARTIFACT_DIR}/appimage-extract"
mkdir -p "${home_dir}/.local/share" "${home_dir}/.config" "${home_dir}/.cache" "${extract_dir}"

echo "[linux-local-truth] installing desktop app from ${INSTALL_URL}" >&2
HOME="${home_dir}" \
XDG_DATA_HOME="${home_dir}/.local/share" \
XDG_CONFIG_HOME="${home_dir}/.config" \
XDG_CACHE_HOME="${home_dir}/.cache" \
PATH="${home_dir}/.local/bin:${PATH}" \
CTX_INSTALL_NO_OPEN=1 \
bash -lc "curl -fsSL '${INSTALL_URL}' | sh" >"${INSTALL_STDOUT}" 2>"${INSTALL_STDERR}"

launcher_path="${home_dir}/.local/bin/ctx-desktop"
desktop_entry_path="${home_dir}/.local/share/applications/ctx.desktop"
icon_path="${home_dir}/.local/share/icons/hicolor/512x512/apps/ctx.png"
app_path="${home_dir}/.local/share/ctx/ctx.AppImage"

for required in "${launcher_path}" "${desktop_entry_path}" "${icon_path}" "${app_path}"; do
  if [[ ! -e "${required}" ]]; then
    write_report "failed" "installer_missing_expected_artifact"
    echo "error: install flow did not create expected artifact: ${required}" >&2
    exit 1
  fi
done

chmod +x "${app_path}"
(
  cd "${extract_dir}"
  "${app_path}" --appimage-extract >/dev/null
)

bundle_manifest="$(find "${extract_dir}/squashfs-root" -type f -path '*/bundles/manifest.json' -print -quit)"
if [[ -z "${bundle_manifest}" ]]; then
  write_report "failed" "appimage_bundle_manifest_missing"
  echo "error: failed to locate bundled manifest.json inside extracted AppImage" >&2
  exit 1
fi
bundle_dir="$(dirname "${bundle_manifest}")"

echo "[linux-local-truth] running installed AppImage through workspace wizard local-codex-smoke" >&2
set +e
HOME="${home_dir}" \
XDG_DATA_HOME="${home_dir}/.local/share" \
XDG_CONFIG_HOME="${home_dir}/.config" \
XDG_CACHE_HOME="${home_dir}/.cache" \
PATH="${home_dir}/.local/bin:${PATH}" \
CTX_AUTOMATION_SHIPPED_APP=1 \
CTX_AUTOMATION_SHIPPED_APP_BUNDLES_DIR="${bundle_dir}" \
CTX_AUTOMATION_SKIP_APP_BUILD=1 \
CTX_AUTOMATION_SKIP_DESKTOP_PREP_RELEASE=1 \
CTX_AUTOMATION_SCENARIOS="${CTX_AUTOMATION_SCENARIOS:-local-codex-smoke}" \
CTX_AUTOMATION_WDIO_LOG_LEVEL="${CTX_AUTOMATION_WDIO_LOG_LEVEL:-warn}" \
CTX_AUTOMATION_KEEP_TMPDIR=1 \
CTX_VOLATILE_ROOT="${ARTIFACT_DIR}/volatile" \
CTX_DESKTOP_APP_PATH="${app_path}" \
"${ROOT}/scripts/desktop_smoke_with_infisical.sh" -- --spec automation/specs/workspace-wizard.spec.cjs >"${WDIO_LOG}" 2>&1
wdio_status=$?
set -e

if [[ "${wdio_status}" -eq 0 ]]; then
  write_report "passed" "workspace_wizard_local_codex_smoke_passed"
  echo "[linux-local-truth] passed; report=${REPORT_PATH}" >&2
  exit 0
fi

failure_reason="workspace_wizard_failed"
if [[ -f "${WDIO_LOG}" ]] && matches_log "local sandbox runtime is unavailable|install nerdctl|CTX_HARNESS_SANDBOX_CLI_PATH"; then
  failure_reason="runtime_bootstrap_missing"
fi

write_report "failed" "${failure_reason}"
tail -n 200 "${WDIO_LOG}" >&2 || true
echo "[linux-local-truth] failed; report=${REPORT_PATH}" >&2
exit "${wdio_status}"
