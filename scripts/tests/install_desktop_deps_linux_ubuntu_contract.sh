#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$ROOT/scripts/install_desktop_deps_linux_ubuntu.sh"

if [[ ! -x "$SCRIPT" ]]; then
  echo "error: missing install script: $SCRIPT" >&2
  exit 2
fi

common_available_packages=(
  libwebkit2gtk-4.1-dev
  libsoup-3.0-dev
  webkit2gtk-driver
  libayatana-appindicator3-dev
)

assert_line_present() {
  local output="$1"
  local expected="$2"
  if ! printf '%s\n' "$output" | grep -Fxq "$expected"; then
    echo "error: expected resolved package '$expected'" >&2
    printf '%s\n' "$output" >&2
    exit 1
  fi
}

assert_line_absent() {
  local output="$1"
  local unexpected="$2"
  if printf '%s\n' "$output" | grep -Fxq "$unexpected"; then
    echo "error: unexpected resolved package '$unexpected'" >&2
    printf '%s\n' "$output" >&2
    exit 1
  fi
}

available_ubuntu_2404=("${common_available_packages[@]}" libfuse2t64)
output_2404="$(
  CTX_TEST_APT_CACHE_AVAILABLE_PACKAGES="${available_ubuntu_2404[*]}" \
    "$SCRIPT" --print-selected-packages
)"
assert_line_present "$output_2404" "libfuse2t64"
assert_line_absent "$output_2404" "libfuse2"
assert_line_absent "$output_2404" "docker-buildx"

available_ubuntu_2204=("${common_available_packages[@]}" libfuse2)
output_2204="$(
  CTX_TEST_APT_CACHE_AVAILABLE_PACKAGES="${available_ubuntu_2204[*]}" \
    "$SCRIPT" --print-selected-packages
)"
assert_line_present "$output_2204" "libfuse2"
assert_line_absent "$output_2204" "docker-buildx"

echo "ok: install_desktop_deps_linux_ubuntu resolves AppImage FUSE runtime packages"
