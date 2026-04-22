#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
core_root="$(cd "${script_dir}/.." && pwd)"
cd "${core_root}"

if [[ -z "${LLVM_COV:-}" || -z "${LLVM_PROFDATA:-}" ]]; then
  host="$(rustc -vV | sed -n 's/^host: //p')"
  sysroot="$(rustc --print sysroot)"
  rustup_llvm_cov="${sysroot}/lib/rustlib/${host}/bin/llvm-cov"
  rustup_llvm_profdata="${sysroot}/lib/rustlib/${host}/bin/llvm-profdata"

  if [[ -x "${rustup_llvm_cov}" && -x "${rustup_llvm_profdata}" ]]; then
    export LLVM_COV="${LLVM_COV:-${rustup_llvm_cov}}"
    export LLVM_PROFDATA="${LLVM_PROFDATA:-${rustup_llvm_profdata}}"
  elif [[ -x /opt/homebrew/opt/llvm/bin/llvm-cov && -x /opt/homebrew/opt/llvm/bin/llvm-profdata ]]; then
    export LLVM_COV="${LLVM_COV:-/opt/homebrew/opt/llvm/bin/llvm-cov}"
    export LLVM_PROFDATA="${LLVM_PROFDATA:-/opt/homebrew/opt/llvm/bin/llvm-profdata}"
  fi
fi

mkdir -p coverage
eval "$(node "${script_dir}/print_ctx_cache_env.cjs" --mode workspace --cwd "${core_root}" --format shell --mkdir)"
coverage_target_dir="${CARGO_TARGET_DIR_COVERAGE:-${CARGO_TARGET_DIR}/coverage/llvm-cov}"
mkdir -p "${coverage_target_dir}"
CARGO_INCREMENTAL=0 \
CARGO_TARGET_DIR="${coverage_target_dir}" \
  node "${script_dir}/run_with_ctx_cache_env.cjs" --mode workspace --cwd "${core_root}" -- \
  cargo llvm-cov --workspace --lcov --output-path coverage/daemon.lcov
