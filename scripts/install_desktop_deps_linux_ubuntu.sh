#!/usr/bin/env bash
set -euo pipefail

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  cat <<'EOF'
Usage: scripts/install_desktop_deps_linux_ubuntu.sh

Installs system packages required to build the Tauri v2 desktop app on Ubuntu/Debian.

This script intentionally does NOT install:
  - Rust toolchain (rustup/cargo)
  - Node.js / pnpm

EOF
  exit 0
fi

if ! command -v apt-get >/dev/null 2>&1; then
  echo "error: apt-get not found; this script targets Ubuntu/Debian." >&2
  exit 1
fi

choose_first_available_pkg() {
  local pkg
  for pkg in "$@"; do
    if apt-cache show "$pkg" >/dev/null 2>&1; then
      printf '%s' "$pkg"
      return 0
    fi
  done
  return 1
}

if [[ -t 1 ]]; then
  BOLD=$'\033[1m'
  RESET=$'\033[0m'
else
  BOLD=""
  RESET=""
fi

SUDO=()
if [[ "$(id -u)" -ne 0 ]]; then
  if ! command -v sudo >/dev/null 2>&1; then
    echo "error: sudo not found and not running as root." >&2
    exit 1
  fi

  if [[ -t 0 ]]; then
    SUDO=("sudo")
  else
    if sudo -n true >/dev/null 2>&1; then
      SUDO=("sudo" "-n")
    else
      cat >&2 <<'EOF'
error: this shell is non-interactive and sudo requires a password.

Run this script from an interactive terminal (so sudo can prompt), or run it as root.
EOF
      exit 1
    fi
  fi
fi

packages=(
  build-essential
  pkg-config
  curl
  file
  xdg-utils
  desktop-file-utils
  squashfs-tools
  zsync
  patchelf
  appstream
  libgtk-3-bin
  libglib2.0-dev
  libgtk-3-dev
  librsvg2-dev
  libssl-dev
  musl
)

echo "${BOLD}Installing desktop (Tauri v2) Linux build dependencies (Ubuntu/Debian)${RESET}"

"${SUDO[@]}" apt-get update

webkit_pkg="$(choose_first_available_pkg libwebkit2gtk-4.1-dev)" || {
  echo "error: could not find libwebkit2gtk-4.1-dev (required by current Tauri Linux stack)." >&2
  exit 3
}

libsoup_pkg="$(choose_first_available_pkg libsoup-3.0-dev)" || {
  echo "error: could not find libsoup-3.0-dev (required by current Tauri Linux stack)." >&2
  exit 3
}

appindicator_pkg="$(choose_first_available_pkg libayatana-appindicator3-dev libappindicator3-dev)" || {
  echo "error: could not find an appindicator dev package (tried libayatana-appindicator3-dev, libappindicator3-dev)." >&2
  exit 3
}

"${SUDO[@]}" env DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
  "${packages[@]}" \
  "$webkit_pkg" \
  "$libsoup_pkg" \
  "$appindicator_pkg"

echo
echo "${BOLD}Sanity check (pkg-config)${RESET}"
for pc in glib-2.0 gtk+-3.0 libsoup-3.0 javascriptcoregtk-4.1 webkit2gtk-4.1; do
  if pkg-config --exists "${pc}"; then
    echo "- ${pc}: OK"
  else
    echo "- ${pc}: MISSING (check packages and pkg-config search path)" >&2
    exit 2
  fi
done

if command -v xdg-mime >/dev/null 2>&1; then
  echo "- xdg-mime: OK ($(command -v xdg-mime))"
else
  echo "- xdg-mime: MISSING (install xdg-utils)" >&2
  exit 2
fi

for cmd in desktop-file-validate mksquashfs zsyncmake patchelf appstreamcli gtk-update-icon-cache; do
  if command -v "$cmd" >/dev/null 2>&1; then
    echo "- $cmd: OK ($(command -v "$cmd"))"
  else
    echo "- $cmd: MISSING (install corresponding AppImage packaging deps)" >&2
    exit 2
  fi
done

echo
echo "${BOLD}Next steps${RESET}"
cat <<'EOF'
- Ensure prerequisites: Rust toolchain + Node.js + pnpm
- Install JS deps:    pnpm -C core install
- Prep web + daemons: pnpm -C core desktop:prep
- Run desktop dev:    pnpm -C core/apps/desktop dev
EOF
