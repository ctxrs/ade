#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
source "$ROOT/scripts/lib/release_daemon_path.sh"

tmp="$(mktemp -d /tmp/ctx-release-daemon-path-lib.XXXXXX)"
cleanup() {
  rm -rf "$tmp"
}
trap cleanup EXIT

bin_dir="$tmp/core/apps/desktop/src-tauri/bin"
mkdir -p "$bin_dir"

explicit_path="$tmp/custom-daemon"
printf 'explicit\n' >"$explicit_path"
RELEASE_DAEMON_PATH="$explicit_path"
ctx_release_export_daemon_path "$tmp" "linux-x64"
if [[ "$RELEASE_DAEMON_PATH" != "$explicit_path" ]]; then
  echo "error: explicit RELEASE_DAEMON_PATH was not preserved" >&2
  exit 1
fi
unset RELEASE_DAEMON_PATH

linux_path="$bin_dir/ctx-daemon-x86_64-unknown-linux-gnu"
printf 'linux\n' >"$linux_path"
RELEASE_PLATFORM=linux-x64 ctx_release_export_daemon_path "$tmp"
if [[ "$RELEASE_DAEMON_PATH" != "$linux_path" ]]; then
  echo "error: linux-x64 daemon path resolved incorrectly: $RELEASE_DAEMON_PATH" >&2
  exit 1
fi
unset RELEASE_DAEMON_PATH RELEASE_PLATFORM

macos_path="$bin_dir/ctx-daemon-aarch64-apple-darwin"
printf 'macos\n' >"$macos_path"
ctx_release_export_daemon_path "$tmp" "macos-arm64"
if [[ "$RELEASE_DAEMON_PATH" != "$macos_path" ]]; then
  echo "error: macos-arm64 daemon path resolved incorrectly: $RELEASE_DAEMON_PATH" >&2
  exit 1
fi
unset RELEASE_DAEMON_PATH

set +e
output="$(ctx_release_export_daemon_path "$tmp" "linux-arm64" 2>&1)"
status=$?
set -e
if [[ "$status" -ne 4 ]]; then
  printf '%s\n' "$output" >&2
  echo "error: expected missing sidecar to fail with exit 4, got $status" >&2
  exit 1
fi
if [[ "$output" != *"missing target-specific daemon sidecar"* || "$output" != *"desktop:prep:release"* ]]; then
  printf '%s\n' "$output" >&2
  echo "error: missing sidecar failure was not actionable" >&2
  exit 1
fi

echo "ok: release daemon path helper resolves target sidecars"
