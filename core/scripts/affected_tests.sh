#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
usage: affected_tests.sh [--base <git_ref>]

Run an affected-tests fast path for agent loops.

Behavior:
- If high-risk paths changed, falls back to full safety gate:
  - pnpm test:agent
  - pnpm verify:e2e
- Otherwise runs a targeted subset based on changed files.
EOF
}

resolve_fast_gate_script() {
  if [[ -n "${CTX_AFFECTED_TESTS_FAST_GATE:-}" ]]; then
    printf '%s\n' "$CTX_AFFECTED_TESTS_FAST_GATE"
    return 0
  fi
  case "$(resolve_uname)" in
    Linux)
      printf '%s\n' 'test:agent:linux-rbe'
      ;;
    *)
      printf '%s\n' 'test:agent'
      ;;
  esac
}

resolve_uname() {
  if [[ -n "${CTX_AFFECTED_TESTS_UNAME:-}" ]]; then
    printf '%s\n' "${CTX_AFFECTED_TESTS_UNAME}"
    return 0
  fi
  uname -s
}

resolve_changed_files() {
  if [[ -n "${CTX_AFFECTED_TESTS_CHANGED_FILES:-}" ]]; then
    printf '%s' "${CTX_AFFECTED_TESTS_CHANGED_FILES}"
    return 0
  fi

  if [[ -n "${base_ref}" ]]; then
    merge_base="$(git -C "${repo_root}" merge-base HEAD "${base_ref}")"
    git -C "${repo_root}" diff --name-only "${merge_base}"...HEAD
    return 0
  fi

  git -C "${repo_root}" diff --name-only HEAD
}

base_ref=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --base)
      if [[ $# -lt 2 ]]; then
        echo "error: --base requires a git ref" >&2
        exit 2
      fi
      base_ref="$2"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "error: unknown option: $1" >&2
      usage >&2
      exit 2
      ;;
  esac
done

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
core_root="$(cd "${script_dir}/.." && pwd)"
repo_root="$(cd "${core_root}/.." && pwd)"
cd "${core_root}"
if [[ "${CTX_AFFECTED_TESTS_SKIP_CACHE_ENV:-0}" != "1" ]]; then
  eval "$(node scripts/print_ctx_cache_env.cjs --mode workspace --format shell --mkdir)"
fi

changed_files="$(resolve_changed_files)"

if [[ -z "${changed_files}" ]]; then
  echo "no changed files detected; running default fast gate"
  pnpm test:agent
  exit 0
fi

echo "changed files:"
echo "${changed_files}" | sed 's/^/  - /'

high_risk=0
needs_premerge=0
needs_web_unit=0
needs_rust=0
declare -a rust_changed_paths=()

while IFS= read -r path; do
  [[ -z "${path}" ]] && continue
  case "${path}" in
    core/apps/web/e2e/*|core/apps/web/e2e/suites/*|core/apps/web/playwright*.ts)
      needs_premerge=1
      ;;
  esac
  case "${path}" in
    core/apps/web/src/*|core/apps/web/package.json|core/apps/web/scripts/*)
      needs_web_unit=1
      ;;
  esac
  case "${path}" in
    core/Cargo.toml|core/Cargo.lock|core/rust-toolchain.toml|core/rustfmt.toml|core/clippy.toml|core/.cargo/config.toml|core/scripts/lib/cache_roots.cjs|core/scripts/lib/ctx_http_suites.cjs|core/scripts/lib/turbo_runner.cjs|core/scripts/lib/rust_workspace_graph.cjs|core/scripts/lib/rust_gate_plan.cjs|core/scripts/ctx_http_suite_task.cjs|core/scripts/rust_crate_task.cjs|core/scripts/run_rust_gate.cjs|core/scripts/run_rust_turbo.cjs|core/scripts/sync_rust_turbo_tasks.cjs|core/crates/*|core/tools/*)
      needs_rust=1
      rust_changed_paths+=("${path}")
      ;;
  esac

  case "${path}" in
    core/crates/ctx-http/*|core/crates/ctx-store/*|core/crates/ctx-mcp/*|core/crates/ctx-providers/*|core/apps/web/src/state/*|core/apps/web/src/api/*|core/apps/web/e2e/*)
      high_risk=1
      ;;
  esac
done <<< "${changed_files}"

run() {
  echo "+ $*"
  if [[ -n "${CTX_AFFECTED_TESTS_COMMAND_LOG:-}" ]]; then
    printf '%s\n' "$*" >> "${CTX_AFFECTED_TESTS_COMMAND_LOG}"
    return 0
  fi
  "$@"
}

run_fast_gate() {
  local fast_gate=""
  fast_gate="$(resolve_fast_gate_script)"
  run pnpm "$fast_gate"
}

if [[ "${high_risk}" -eq 1 ]]; then
  echo "high-risk paths changed; using full safety fallback"
  run_fast_gate
  run pnpm verify:e2e
  exit 0
fi

ran_any=0
if [[ "${needs_rust}" -eq 1 ]]; then
  run pnpm rust:turbo:check
  rust_args=(exec node scripts/run_rust_gate.cjs --mode workspace --include-reverse-deps --clippy --test-strategy mixed)
  for changed_path in "${rust_changed_paths[@]}"; do
    rust_args+=(--changed-file "${changed_path}")
  done
  run pnpm "${rust_args[@]}"
  ran_any=1
fi

if [[ "${needs_web_unit}" -eq 1 ]]; then
  run pnpm -C apps/web test:quiet
  ran_any=1
fi

if [[ "${needs_premerge}" -eq 1 ]]; then
  run pnpm verify:e2e
  ran_any=1
fi

if [[ "${ran_any}" -eq 0 ]]; then
  echo "no targeted mapping hit; running default fast gate"
  run_fast_gate
fi
