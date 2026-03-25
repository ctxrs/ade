#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PROFILE="${CTX_CRP_PROFILE:-debug}"
WORKSPACE="${ROOT_DIR}/external-harnesses/codex/codex-rs"

# shellcheck source=lib/codex_crp_build_env.sh
source "${ROOT_DIR}/scripts/lib/codex_crp_build_env.sh"

TARGET_DIR="$(codex_crp_target_dir "${ROOT_DIR}")"
BUILD_JOBS="$(codex_crp_build_jobs)"

export CARGO_TARGET_DIR="${TARGET_DIR}"
export CARGO_BUILD_JOBS="${BUILD_JOBS}"

mkdir -p "${TARGET_DIR}"

cd "${WORKSPACE}"
if [[ "${PROFILE}" == "release" ]]; then
  cargo test -q --release -p codex-crp
else
  cargo test -q -p codex-crp
fi
