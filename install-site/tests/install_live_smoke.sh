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
need_cmd python3

tmp="$(mktemp -d /tmp/ctx-install-live-smoke.XXXXXX)"
home_dir="$tmp/home"
log_path="$tmp/install.log"
manifest_path="$tmp/latest.json"
trap 'rm -rf "$tmp"' EXIT

mkdir -p "$home_dir/.local/share"

curl -fsSL "https://api.ctx.rs/functions/v1/releases/stable/latest.json" -o "$manifest_path"

linux_has_deb="$(
  python3 - "$manifest_path" <<'PY'
import json
import sys

with open(sys.argv[1], "r", encoding="utf-8") as handle:
    manifest = json.load(handle)

linux_entry = manifest.get("platforms", {}).get("linux-x64", {})
print("1" if isinstance(linux_entry.get("deb"), dict) else "0")
PY
)"

HOME="$home_dir" \
XDG_DATA_HOME="$home_dir/.local/share" \
PATH="$home_dir/.local/bin:$PATH" \
CTX_INSTALL_NO_OPEN=1 \
bash -lc 'curl -fsSL https://ctx.rs/install | sh' > /dev/null 2>"$log_path"

if [[ "$linux_has_deb" == "1" ]]; then
  grep -F "Installed ctx desktop Debian package" "$log_path" >/dev/null
  echo "ok: live installer preferred Debian package on Ubuntu"
  exit 0
fi

launcher_path="$home_dir/.local/bin/ctx-desktop"
desktop_entry_path="$home_dir/.local/share/applications/ctx.desktop"

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

grep -F "export CTX_DESKTOP_START_PATH=/" "$launcher_path" >/dev/null
grep -F "Exec=$launcher_path" "$desktop_entry_path" >/dev/null

echo "ok: live installer created AppImage launcher wrapper + desktop entry"
