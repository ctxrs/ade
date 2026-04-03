#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

if [[ "$(uname -s)" != "Linux" ]]; then
  echo "skip: live installer smoke only validates Linux in this workflow"
  exit 0
fi

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "error: missing required command: $1" >&2
    exit 1
  }
}

need_cmd curl

tmp="$(mktemp -d /tmp/ctx-install-live-smoke.XXXXXX)"
home_dir="$tmp/home"
log_path="$tmp/install.log"
trap 'rm -rf "$tmp"' EXIT

mkdir -p "$home_dir/.local/share"

HOME="$home_dir" \
XDG_DATA_HOME="$home_dir/.local/share" \
PATH="$home_dir/.local/bin:$PATH" \
CTX_INSTALL_NO_OPEN=1 \
bash -lc 'curl -fsSL https://ctx.rs/install | sh' > /dev/null 2>"$log_path"

launcher_path="$home_dir/.local/bin/ctx-desktop"
desktop_entry_path="$home_dir/.local/share/applications/ctx.desktop"
icon_path="$home_dir/.local/share/icons/hicolor/512x512/apps/ctx.png"

if [[ ! -f "$launcher_path" ]]; then
  echo "error: live installer did not create launcher wrapper at $launcher_path" >&2
  cat "$log_path" >&2 || true
  exit 1
fi

if [[ ! -f "$desktop_entry_path" ]]; then
  echo "error: live installer did not create desktop entry at $desktop_entry_path" >&2
  cat "$log_path" >&2 || true
  exit 1
fi

if [[ ! -f "$icon_path" ]]; then
  echo "error: live installer did not create icon at $icon_path" >&2
  cat "$log_path" >&2 || true
  exit 1
fi

grep -F "export CTX_DESKTOP_START_PATH=/" "$launcher_path" >/dev/null
grep -F "Exec=$launcher_path" "$desktop_entry_path" >/dev/null
grep -F "Icon=$icon_path" "$desktop_entry_path" >/dev/null

echo "ok: live installer created AppImage launcher wrapper + desktop entry + icon"
