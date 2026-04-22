#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")"/../../.. && pwd)"
TAURI_DIR="${ROOT}/apps/desktop/src-tauri"
WEB_DIR="${TAURI_DIR}/web"
WEB_DIST="${WEB_DIR}/dist"
CREATED_WEB_DIR=0
CREATED_WEB_DIST=0

cleanup() {
  if [[ "${CREATED_WEB_DIST}" == "1" ]]; then
    rm -rf "${WEB_DIST}"
  fi
  if [[ "${CREATED_WEB_DIR}" == "1" ]]; then
    rmdir "${WEB_DIR}" 2>/dev/null || true
  fi
}

trap cleanup EXIT

if [[ ! -d "${WEB_DIR}" ]]; then
  mkdir -p "${WEB_DIST}"
  CREATED_WEB_DIR=1
  CREATED_WEB_DIST=1
elif [[ ! -d "${WEB_DIST}" ]]; then
  mkdir -p "${WEB_DIST}"
  CREATED_WEB_DIST=1
fi

export CTX_DESKTOP_SKIP_TAURI_BUILD="${CTX_DESKTOP_SKIP_TAURI_BUILD:-1}"

if rg -n "mac_notification_sys" "${TAURI_DIR}/src/desktop_notifications.rs"; then
  echo "desktop notification code must not import mac_notification_sys" >&2
  exit 1
fi

if rg -n "mac-notification-sys" "${TAURI_DIR}/Cargo.toml"; then
  echo "desktop macOS notifications must not depend on mac-notification-sys" >&2
  exit 1
fi

if rg -n "tauri_plugin_notification::init|tauri-plugin-notification" "${TAURI_DIR}/src/main.rs" "${TAURI_DIR}/Cargo.toml"; then
  echo "desktop notifications must use the ctx-owned IPC backend, not tauri-plugin-notification" >&2
  exit 1
fi

if rg -n '"notification:default"' "${TAURI_DIR}/capabilities/default.json"; then
  echo "desktop capabilities must not expose the unused Tauri notification plugin capability" >&2
  exit 1
fi

cargo test --manifest-path "${TAURI_DIR}/Cargo.toml" desktop_attention::tests::
cargo test --manifest-path "${TAURI_DIR}/Cargo.toml" desktop_notifications::tests::
cargo test --manifest-path "${TAURI_DIR}/Cargo.toml" desktop_deeplink::deep_link_parse_tests::
cargo test --manifest-path "${TAURI_DIR}/Cargo.toml" desktop_windows::tests::
