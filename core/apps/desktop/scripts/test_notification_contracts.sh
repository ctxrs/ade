#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")"/../../.. && pwd)"
TAURI_DIR="${ROOT}/apps/desktop/src-tauri"
WEB_DIR="${TAURI_DIR}/web"
WEB_DIST="${WEB_DIR}/dist"
CREATED_WEB_DIR=0

cleanup() {
  if [[ "${CREATED_WEB_DIR}" == "1" ]]; then
    rm -rf "${WEB_DIR}"
  fi
}

trap cleanup EXIT

if [[ ! -d "${WEB_DIST}" ]]; then
  mkdir -p "${WEB_DIST}"
  CREATED_WEB_DIR=1
fi

export CTX_DESKTOP_SKIP_TAURI_BUILD="${CTX_DESKTOP_SKIP_TAURI_BUILD:-1}"

cargo test --manifest-path "${TAURI_DIR}/Cargo.toml" desktop_attention::tests::
cargo test --manifest-path "${TAURI_DIR}/Cargo.toml" desktop_notifications::tests::
cargo test --manifest-path "${TAURI_DIR}/Cargo.toml" desktop_deeplink::deep_link_parse_tests::
cargo test --manifest-path "${TAURI_DIR}/Cargo.toml" desktop_windows::tests::
