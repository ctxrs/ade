#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source "$ROOT/scripts/lib/updater_e2e_drill_full_paths.sh"

tmp="$(mktemp -d /tmp/ctx-updater-linux-smoke-contract.XXXXXX)"
stderr_one="$tmp/stderr-appimage.txt"
stderr_two="$tmp/stderr-missing-appdir.txt"
trap 'rm -rf "$tmp"' EXIT

bundle_dir="$tmp/core/apps/desktop/src-tauri/target/release/bundle/appimage"
fallback_bundle_dir="$tmp/core/target/release/bundle/appimage"
apprun_path="$bundle_dir/ctx_0.5.18_amd64.AppDir/AppRun"
apprun_wrapped_path="${apprun_path}.wrapped"
appdir_app_bin="$bundle_dir/ctx_0.5.18_amd64.AppDir/usr/bin/ctx"
appdir_daemon_bin="$bundle_dir/ctx_0.5.18_amd64.AppDir/usr/bin/ctx-daemon"
webkit_network_process="$bundle_dir/ctx_0.5.18_amd64.AppDir/lib/x86_64-linux-gnu/webkit2gtk-4.1/WebKitNetworkProcess"
fallback_apprun_path="$fallback_bundle_dir/ctx_0.5.18_amd64.AppDir/AppRun"
extracted_app_dir="$tmp/extracted/squashfs-root"
extracted_apprun_path="$extracted_app_dir/AppRun"
extracted_app_bin="$extracted_app_dir/usr/bin/ctx"
extracted_webkit_process="$extracted_app_dir/lib/x86_64-linux-gnu/webkit2gtk-4.1/WebKitWebProcess"
appimage_path="$bundle_dir/ctx_0.5.18_amd64.AppImage"

mkdir -p "$(dirname "$apprun_path")" "$(dirname "$appdir_app_bin")" "$(dirname "$webkit_network_process")"
mkdir -p "$(dirname "$extracted_apprun_path")" "$(dirname "$extracted_app_bin")" "$(dirname "$extracted_webkit_process")"
printf '#!/usr/bin/env bash\nexit 0\n' >"$apprun_path"
chmod 0644 "$apprun_path"
printf '#!/usr/bin/env bash\nexit 0\n' >"$apprun_wrapped_path"
chmod 0644 "$apprun_wrapped_path"
printf '#!/usr/bin/env bash\nexit 0\n' >"$appdir_app_bin"
chmod 0644 "$appdir_app_bin"
printf '#!/usr/bin/env bash\nexit 0\n' >"$appdir_daemon_bin"
chmod 0644 "$appdir_daemon_bin"
printf '#!/usr/bin/env bash\nexit 0\n' >"$webkit_network_process"
chmod 0644 "$webkit_network_process"
printf '#!/usr/bin/env bash\nexit 0\n' >"$extracted_apprun_path"
chmod 0644 "$extracted_apprun_path"
printf '#!/usr/bin/env bash\nexit 0\n' >"$extracted_app_bin"
chmod 0644 "$extracted_app_bin"
printf '#!/usr/bin/env bash\nexit 0\n' >"$extracted_webkit_process"
chmod 0644 "$extracted_webkit_process"
printf 'not-a-real-appimage\n' >"$appimage_path"
chmod +x "$appimage_path"

resolved_path="$(resolve_local_smoke_app_path "$tmp" Linux)"
if [[ "$resolved_path" != "$apprun_path" ]]; then
  echo "error: expected Linux resolver to return AppDir AppRun launcher" >&2
  echo "expected: $apprun_path" >&2
  echo "actual:   $resolved_path" >&2
  exit 1
fi

normalize_local_smoke_app_permissions "$resolved_path"
if [[ ! -x "$apprun_path" ]]; then
  echo "error: expected AppRun launcher to be normalized executable" >&2
  exit 1
fi
if [[ ! -x "$apprun_wrapped_path" ]]; then
  echo "error: expected AppRun.wrapped helper to be normalized executable" >&2
  exit 1
fi
if [[ ! -x "$appdir_app_bin" ]]; then
  echo "error: expected AppDir app launcher binary to be normalized executable" >&2
  exit 1
fi
if [[ ! -x "$appdir_daemon_bin" ]]; then
  echo "error: expected AppDir daemon sidecar to be normalized executable" >&2
  exit 1
fi
if [[ ! -x "$webkit_network_process" ]]; then
  echo "error: expected nested WebKit helper to be normalized executable" >&2
  exit 1
fi
if ! CTX_DESKTOP_APP_PATH="$extracted_apprun_path" resolve_local_smoke_app_path "$tmp" Linux >/dev/null; then
  echo "error: expected extracted AppImage AppRun override to be accepted" >&2
  exit 1
fi
normalize_local_smoke_app_permissions "$extracted_apprun_path"
if [[ ! -x "$extracted_apprun_path" || ! -x "$extracted_app_bin" || ! -x "$extracted_webkit_process" ]]; then
  echo "error: expected extracted AppImage tree helpers to be normalized executable" >&2
  exit 1
fi

rm -rf "$bundle_dir"
mkdir -p "$(dirname "$fallback_apprun_path")"
printf '#!/usr/bin/env bash\nexit 0\n' >"$fallback_apprun_path"
chmod 0644 "$fallback_apprun_path"
resolved_path="$(resolve_local_smoke_app_path "$tmp" Linux)"
if [[ "$resolved_path" != "$fallback_apprun_path" ]]; then
  echo "error: expected Linux resolver to return fallback core/target AppDir AppRun launcher" >&2
  echo "expected: $fallback_apprun_path" >&2
  echo "actual:   $resolved_path" >&2
  exit 1
fi
normalize_local_smoke_app_permissions "$resolved_path"
if [[ ! -x "$fallback_apprun_path" ]]; then
  echo "error: expected fallback AppRun launcher to be normalized executable" >&2
  exit 1
fi

rm -rf "$fallback_bundle_dir"
mkdir -p "$(dirname "$appimage_path")"
printf 'not-a-real-appimage\n' >"$appimage_path"
chmod +x "$appimage_path"

if CTX_DESKTOP_APP_PATH="$appimage_path" resolve_local_smoke_app_path "$tmp" Linux > /dev/null 2>"$stderr_one"; then
  echo "error: expected AppImage override to be rejected" >&2
  exit 1
fi

if ! grep -Fq "outer AppImage wrapper" "$stderr_one"; then
  echo "error: expected AppImage rejection message" >&2
  cat "$stderr_one" >&2 || true
  exit 1
fi

mkdir -p "$(dirname "$appdir_daemon_bin")"
printf '#!/usr/bin/env bash\nexit 0\n' >"$appdir_daemon_bin"
chmod 0644 "$appdir_daemon_bin"

if CTX_DESKTOP_APP_PATH="$appdir_daemon_bin" resolve_local_smoke_app_path "$tmp" Linux > /dev/null 2>"$stderr_two"; then
  echo "error: expected inner AppDir binary override to be rejected" >&2
  exit 1
fi

if ! grep -Fq "inner bundled binary" "$stderr_two"; then
  echo "error: expected inner AppDir binary rejection message" >&2
  cat "$stderr_two" >&2 || true
  exit 1
fi

rm -f "$apprun_path"

if resolve_local_smoke_app_path "$tmp" Linux > /dev/null 2>"$stderr_two"; then
  echo "error: expected Linux resolver to fail when only AppImage is present" >&2
  exit 1
fi

if ! grep -Fq "*.AppDir/AppRun" "$stderr_two"; then
  echo "error: expected missing AppRun contract message" >&2
  cat "$stderr_two" >&2 || true
  exit 1
fi

echo "ok: updater_e2e_drill_full Linux smoke path contract holds"
