#!/usr/bin/env bash

codex_crp_target_dir() {
  local root_dir="$1"
  local root_basename

  if [[ -n "${CTX_CRP_TARGET_DIR:-}" ]]; then
    printf '%s' "${CTX_CRP_TARGET_DIR}"
    return
  fi

  root_basename="$(basename "${root_dir}")"
  local volatile_targets_dir="${CTX_VOLATILE_TARGETS_DIR:-${HOME}/.ctx/volatile/targets}"
  printf '%s' "${volatile_targets_dir}/ctx-monorepo/codex-crp-${root_basename}"
}

codex_crp_build_jobs() {
  if [[ -n "${CTX_CRP_CARGO_BUILD_JOBS:-}" ]]; then
    printf '%s' "${CTX_CRP_CARGO_BUILD_JOBS}"
    return
  fi

  if [[ -n "${CARGO_BUILD_JOBS:-}" ]]; then
    printf '%s' "${CARGO_BUILD_JOBS}"
    return
  fi

  printf '1'
}
