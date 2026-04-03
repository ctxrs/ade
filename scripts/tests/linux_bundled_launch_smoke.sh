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

echo "smoke: preparing release resources"
pnpm -C core desktop:prep:release

echo "smoke: building Linux bundled desktop artifact"
pnpm -C core/apps/desktop run build -- --bundles appimage -- --features automation

app_path="$(resolve_local_smoke_app_path "$ROOT" Linux)"
normalize_local_smoke_app_permissions "$app_path"

echo "smoke: probing bundled app launch via tauri-driver"
node "$ROOT/core/apps/desktop/scripts/linux_bundled_launch_smoke.mjs" --app "$app_path"
