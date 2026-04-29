#!/usr/bin/env bash

ctx_release_detect_platform() {
  local os_raw arch_raw os_key arch_key
  os_raw="$(uname -s)"
  arch_raw="$(uname -m)"
  case "$os_raw" in
    Linux*) os_key="linux" ;;
    Darwin*) os_key="macos" ;;
    MINGW*|MSYS*|CYGWIN*) os_key="windows" ;;
    *)
      echo "error: unsupported host OS for release publish: $os_raw" >&2
      return 3
      ;;
  esac
  case "$arch_raw" in
    x86_64|amd64) arch_key="x64" ;;
    aarch64|arm64) arch_key="arm64" ;;
    *)
      echo "error: unsupported host arch for release publish: $arch_raw" >&2
      return 3
      ;;
  esac
  if [[ "$os_key" == "windows" && "$arch_key" != "x64" ]]; then
    echo "error: unsupported windows arch for release publish: $arch_raw" >&2
    return 3
  fi
  printf '%s-%s\n' "$os_key" "$arch_key"
}

ctx_release_daemon_target_triple() {
  local platform="$1"
  case "$platform" in
    linux-x64) printf '%s\n' 'x86_64-unknown-linux-gnu' ;;
    linux-arm64) printf '%s\n' 'aarch64-unknown-linux-gnu' ;;
    macos-x64) printf '%s\n' 'x86_64-apple-darwin' ;;
    macos-arm64) printf '%s\n' 'aarch64-apple-darwin' ;;
    windows-x64) printf '%s\n' 'x86_64-pc-windows-msvc' ;;
    *)
      echo "error: unsupported release platform for daemon sidecar: $platform" >&2
      return 3
      ;;
  esac
}

ctx_release_daemon_extension() {
  local platform="$1"
  case "$platform" in
    windows-*) printf '%s\n' '.exe' ;;
    *) printf '%s\n' '' ;;
  esac
}

ctx_release_resolve_daemon_path() {
  local root="$1"
  local platform="${2:-}"
  if [[ -n "${RELEASE_DAEMON_PATH:-}" ]]; then
    printf '%s\n' "$RELEASE_DAEMON_PATH"
    return 0
  fi
  if [[ -z "$platform" ]]; then
    platform="${RELEASE_PLATFORM:-}"
  fi
  if [[ -z "$platform" ]]; then
    platform="$(ctx_release_detect_platform)" || return $?
  fi

  local target_triple ext
  target_triple="$(ctx_release_daemon_target_triple "$platform")" || return $?
  ext="$(ctx_release_daemon_extension "$platform")" || return $?
  printf '%s\n' "$root/core/apps/desktop/src-tauri/bin/ctx-daemon-${target_triple}${ext}"
}

ctx_release_export_daemon_path() {
  local root="$1"
  local platform="${2:-}"
  if [[ -n "${RELEASE_DAEMON_PATH:-}" ]]; then
    export RELEASE_DAEMON_PATH
    return 0
  fi

  local daemon_path
  daemon_path="$(ctx_release_resolve_daemon_path "$root" "$platform")" || return $?
  if [[ ! -f "$daemon_path" ]]; then
    echo "error: missing target-specific daemon sidecar at $daemon_path" >&2
    echo "hint: run 'pnpm -C core desktop:prep:release' or set RELEASE_DAEMON_PATH explicitly." >&2
    return 4
  fi
  export RELEASE_DAEMON_PATH="$daemon_path"
}
