#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 2 ]]; then
  cat >&2 <<'EOF'
usage: scripts/tests/release_linux_image_smoke.sh <image-ref> <platform>

examples:
  scripts/tests/release_linux_image_smoke.sh ghcr.io/ctxrs/ctx-release-linux:sha-abc123 linux/amd64
  scripts/tests/release_linux_image_smoke.sh ghcr.io/ctxrs/ctx-release-linux@sha256:... linux/arm64
EOF
  exit 2
fi

image_ref="$1"
platform="$2"

for cmd in docker; do
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "error: required command not found: $cmd" >&2
    exit 1
  fi
done

docker run --rm --platform "$platform" "$image_ref" bash -c '
  set -euo pipefail

  required_cmds=(
    appstreamcli
    cargo
    desktop-file-validate
    file
    gtk-update-icon-cache
    ldd
    mksquashfs
    node
    patchelf
    pnpm
    python3
    readelf
    rustc
    xdg-mime
    zsyncmake
  )
  for c in "${required_cmds[@]}"; do
    command -v "$c" >/dev/null 2>&1 || { echo "missing command: $c" >&2; exit 1; }
  done

  required_pc=(
    glib-2.0
    gtk+-3.0
    libsoup-3.0
    javascriptcoregtk-4.1
    webkit2gtk-4.1
  )
  for pc in "${required_pc[@]}"; do
    pkg-config --exists "$pc" || { echo "missing pkg-config target: $pc" >&2; exit 1; }
  done
'
