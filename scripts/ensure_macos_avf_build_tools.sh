#!/usr/bin/env bash
set -euo pipefail

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "error: ensure_macos_avf_build_tools.sh only supports Darwin hosts" >&2
  exit 2
fi

if ! command -v brew >/dev/null 2>&1; then
  echo "error: Homebrew is required to provision macOS AVF build tools" >&2
  exit 2
fi

if ! command -v qemu-img >/dev/null 2>&1; then
  brew install qemu
fi

if ! command -v zig >/dev/null 2>&1; then
  brew install zig
fi

if ! command -v cargo-zigbuild >/dev/null 2>&1; then
  cargo install cargo-zigbuild --locked
fi
