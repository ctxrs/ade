#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "error: linux bundled launch smoke requires Linux" >&2
  exit 1
fi

if ! command -v pnpm >/dev/null 2>&1; then
  echo "error: pnpm is required" >&2
  exit 1
fi

source "$ROOT/scripts/lib/updater_e2e_drill_full_paths.sh"

sanitize_smoke_path_list() {
  local value="${1:-}"
  local cleaned=""
  local old_ifs="$IFS"
  local segment
  IFS=:
  for segment in $value; do
    case "$segment" in
      "" | /tmp/.mount_* | /tmp/.mount_*/*)
        continue
        ;;
    esac
    if [[ -n "$cleaned" ]]; then
      cleaned+=":$segment"
    else
      cleaned="$segment"
    fi
  done
  IFS="$old_ifs"
  printf '%s' "$cleaned"
}

sanitize_smoke_path_env_var() {
  local name="$1"
  local value="${!name:-}"
  local cleaned
  cleaned="$(sanitize_smoke_path_list "$value")"
  if [[ -n "$cleaned" ]]; then
    export "$name=$cleaned"
  else
    unset "$name"
  fi
}

sanitize_smoke_appimage_runtime_env() {
  local var
  for var in PATH LD_LIBRARY_PATH GIO_EXTRA_MODULES GIO_MODULE_DIR GTK_PATH GI_TYPELIB_PATH XDG_DATA_DIRS; do
    sanitize_smoke_path_env_var "$var"
  done
  if [[ -z "${PATH:-}" ]]; then
    export PATH="/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
  fi
  unset APPDIR APPIMAGE ARGV0 CTX_APPIMAGE_PATH CTX_BUNDLE_DIR WEBKIT_EXEC_PATH
}

sanitize_smoke_appimage_runtime_env

smoke_platform="${RELEASE_PLATFORM:-}"
if [[ -z "$smoke_platform" ]]; then
  case "$(uname -m)" in
    x86_64|amd64) smoke_platform="linux-x64" ;;
    aarch64|arm64) smoke_platform="linux-arm64" ;;
    *)
      echo "error: unable to infer linux bundled launch smoke platform from uname -m: $(uname -m)" >&2
      exit 1
      ;;
  esac
fi

echo "smoke: preparing release resources"
pnpm -C core desktop:prep:release

echo "smoke: building Linux bundled desktop artifact"
RELEASE_PLATFORM="$smoke_platform" \
  RELEASE_TAURI_FEATURES=automation \
  CTX_DESKTOP_SKIP_PREP=1 \
  bash scripts/release_linux_tauri_bundle_in_container.sh

app_path="$(resolve_local_smoke_app_path "$ROOT" Linux)"
normalize_local_smoke_app_permissions "$app_path"

echo "smoke: probing bundled app launch via tauri-driver"
node "$ROOT/core/apps/desktop/scripts/linux_bundled_launch_smoke.mjs" --app "$app_path"
