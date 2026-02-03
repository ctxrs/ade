#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PROFILE="${CTX_CRP_PROFILE:-debug}"
WORKSPACE="${ROOT_DIR}/external-harnesses/codex/codex-rs"
TARGET_DIR="${CTX_CRP_TARGET_DIR:-${WORKSPACE}/target}"

export CARGO_TARGET_DIR="${TARGET_DIR}"

cd "${WORKSPACE}"
if [[ "${PROFILE}" == "release" ]]; then
  cargo build -p codex-crp --release
else
  cargo build -p codex-crp
fi
