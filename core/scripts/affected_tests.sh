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
eval "$(node scripts/print_ctx_cache_env.cjs --mode workspace --format shell --mkdir)"

if [[ -n "${base_ref}" ]]; then
  merge_base="$(git -C "${repo_root}" merge-base HEAD "${base_ref}")"
  changed_files="$(git -C "${repo_root}" diff --name-only "${merge_base}"...HEAD)"
else
  changed_files="$(git -C "${repo_root}" diff --name-only HEAD)"
fi

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
declare -a rust_crates=()

contains_crate() {
  local needle="$1"
  shift
  for x in "$@"; do
    if [[ "$x" == "$needle" ]]; then
      return 0
    fi
  done
  return 1
}

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
    core/crates/*)
      needs_rust=1
      crate_name="$(printf '%s' "${path}" | cut -d/ -f3)"
      if ! contains_crate "${crate_name}" "${rust_crates[@]:-}"; then
        rust_crates+=("${crate_name}")
      fi
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
  "$@"
}

if [[ "${high_risk}" -eq 1 ]]; then
  echo "high-risk paths changed; using full safety fallback"
  run pnpm test:agent
  run pnpm verify:e2e
  exit 0
fi

ran_any=0
if [[ "${needs_rust}" -eq 1 ]]; then
  for crate in "${rust_crates[@]}"; do
    run cargo test -q -p "${crate}"
    ran_any=1
  done
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
  run pnpm test:agent
fi
