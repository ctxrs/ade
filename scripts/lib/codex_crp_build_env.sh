#!/usr/bin/env bash

codex_crp_session_id() {
  local root_dir="$1"
  local scope="${2:-codex-crp}"
  local root_basename

  if [[ -n "${CTX_SESSION_ID:-}" ]]; then
    printf '%s' "${CTX_SESSION_ID}"
    return
  fi

  root_basename="$(basename "${root_dir}")"
  printf '%s' "${scope}-${root_basename}-$$"
}

codex_crp_export_cache_env() {
  local root_dir="$1"
  local scope="${2:-codex-crp}"
  local core_dir="${root_dir}/core"

  export CTX_SESSION_ID
  CTX_SESSION_ID="$(codex_crp_session_id "${root_dir}" "${scope}")"
  eval "$(node "${core_dir}/scripts/print_ctx_cache_env.cjs" --mode workspace --cwd "${core_dir}" --format shell --mkdir)"

  if [[ -n "${CTX_CRP_TARGET_DIR:-}" ]]; then
    export CARGO_TARGET_DIR="${CTX_CRP_TARGET_DIR}"
    mkdir -p "${CARGO_TARGET_DIR}"
  fi
}

codex_crp_target_dir() {
  local root_dir="$1"

  if [[ -n "${CTX_CRP_TARGET_DIR:-}" ]]; then
    printf '%s' "${CTX_CRP_TARGET_DIR}"
    return
  fi

  if [[ -n "${CARGO_TARGET_DIR:-}" ]]; then
    printf '%s' "${CARGO_TARGET_DIR}"
    return
  fi

  local volatile_targets_dir="${CTX_VOLATILE_TARGETS_DIR:-${HOME}/.ctx/volatile/targets}"
  printf '%s' "${volatile_targets_dir}/ctx-monorepo/$(codex_crp_session_id "${root_dir}" "codex-crp")"
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

codex_crp_run_rust() {
  local root_dir="$1"
  shift
  node "${root_dir}/core/scripts/run_with_ctx_cache_env.cjs" --mode workspace --cwd "${root_dir}/core" -- cargo "$@"
}
