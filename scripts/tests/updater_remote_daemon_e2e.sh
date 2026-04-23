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

if [[ -z "${CTX_DESKTOP_APP_PATH:-}" ]]; then
  if ! resolved_app_path="$(resolve_local_smoke_app_path "${ROOT}")"; then
    download_base_url="$(resolve_download_base_url)"
    target_channel="$(resolve_target_channel)"
    echo "[updater-remote-proof] local AppDir unavailable; downloading published controller AppImage for ${target_channel}" >&2
    resolved_app_path="$(download_published_controller_app "$download_base_url" "$target_channel")"
  fi
  export CTX_DESKTOP_APP_PATH="${resolved_app_path}"
fi
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
unset CTX_BUNDLE_DIR

pnpm -C "${ROOT}/core/apps/desktop" test:automation:updater-remote
