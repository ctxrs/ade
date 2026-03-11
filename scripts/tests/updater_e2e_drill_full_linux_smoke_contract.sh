#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source "$ROOT/scripts/lib/updater_e2e_drill_full_paths.sh"

tmp="$(mktemp -d /tmp/ctx-updater-linux-smoke-contract.XXXXXX)"
stderr_one="$tmp/stderr-appimage.txt"
stderr_two="$tmp/stderr-missing-appdir.txt"
trap 'rm -rf "$tmp"' EXIT

bundle_dir="$tmp/core/apps/desktop/src-tauri/target/release/bundle/appimage"
apprun_path="$bundle_dir/ctx_0.5.18_amd64.AppDir/AppRun"
apprun_wrapped_path="${apprun_path}.wrapped"
appdir_bin="$bundle_dir/ctx_0.5.18_amd64.AppDir/usr/bin/ctx"
appimage_path="$bundle_dir/ctx_0.5.18_amd64.AppImage"

mkdir -p "$(dirname "$apprun_path")" "$(dirname "$appdir_bin")"
printf '#!/usr/bin/env bash\nexit 0\n' >"$apprun_path"
chmod 0644 "$apprun_path"
printf '#!/usr/bin/env bash\nexit 0\n' >"$apprun_wrapped_path"
chmod 0644 "$apprun_wrapped_path"
printf '#!/usr/bin/env bash\nexit 0\n' >"$appdir_bin"
chmod 0644 "$appdir_bin"
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
if [[ ! -x "$appdir_bin" ]]; then
  echo "error: expected inner AppDir launcher binary to be normalized executable" >&2
  exit 1
fi

if CTX_DESKTOP_APP_PATH="$appimage_path" resolve_local_smoke_app_path "$tmp" Linux > /dev/null 2>"$stderr_one"; then
  echo "error: expected AppImage override to be rejected" >&2
  exit 1
fi

if ! grep -Fq "outer AppImage wrapper" "$stderr_one"; then
  echo "error: expected AppImage rejection message" >&2
  cat "$stderr_one" >&2 || true
  exit 1
fi

if CTX_DESKTOP_APP_PATH="$appdir_bin" resolve_local_smoke_app_path "$tmp" Linux > /dev/null 2>"$stderr_two"; then
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
