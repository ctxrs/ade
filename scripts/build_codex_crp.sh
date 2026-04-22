#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PROFILE="${CTX_CRP_PROFILE:-debug}"
WORKSPACE_MANIFEST="${ROOT_DIR}/core/Cargo.toml"

# shellcheck source=lib/codex_crp_build_env.sh
source "${ROOT_DIR}/scripts/lib/codex_crp_build_env.sh"

TARGET_DIR="$(codex_crp_target_dir "${ROOT_DIR}")"
BUILD_JOBS="$(codex_crp_build_jobs)"

export CARGO_TARGET_DIR="${TARGET_DIR}"
export CARGO_BUILD_JOBS="${BUILD_JOBS}"

mkdir -p "${TARGET_DIR}"

if [[ "${PROFILE}" == "release" ]]; then
  node "${ROOT_DIR}/core/scripts/run_with_ctx_cache_env.cjs" --mode workspace --cwd "${ROOT_DIR}/core" -- \
    cargo build --manifest-path "${WORKSPACE_MANIFEST}" -p codex-crp --release
else
  node "${ROOT_DIR}/core/scripts/run_with_ctx_cache_env.cjs" --mode workspace --cwd "${ROOT_DIR}/core" -- \
    cargo build --manifest-path "${WORKSPACE_MANIFEST}" -p codex-crp
fi
