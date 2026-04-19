#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
usage: affected_tests.sh [--base <git_ref>]
                         [--profile <taxonomy_profile>]

Run an affected-tests fast path for agent loops.

Behavior:
- Uses the testing taxonomy registry to select the affected execution plan for
  the changed files. Defaults to `agent-default`; set `--profile` or
  `CTX_AFFECTED_TESTS_PROFILE` to override.
- Falls back to the minimal taxonomy gate only when the taxonomy plan resolves to no commands.
EOF
}

resolve_fast_gate_script() {
  if [[ -n "${CTX_AFFECTED_TESTS_FAST_GATE:-}" ]]; then
    printf '%s\n' "$CTX_AFFECTED_TESTS_FAST_GATE"
    return 0
  fi
  case "$(resolve_uname)" in
    Linux)
      printf '%s\n' 'test:agent:minimal:linux-rbe'
      ;;
    *)
      printf '%s\n' 'test:agent:minimal'
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
  if [[ "${CTX_AFFECTED_TESTS_CHANGED_FILES+x}" == "x" ]]; then
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
taxonomy_profile="${CTX_AFFECTED_TESTS_PROFILE:-agent-default}"
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
    --profile)
      if [[ $# -lt 2 ]]; then
        echo "error: --profile requires a taxonomy profile id" >&2
        exit 2
      fi
      taxonomy_profile="$2"
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

if [[ -z "${changed_files}" ]]; then
  echo "no changed files detected; running minimal taxonomy gate"
  run_fast_gate
  exit 0
fi

echo "changed files:"
echo "${changed_files}" | sed 's/^/  - /'
echo "taxonomy profile: ${taxonomy_profile}"

declare -a taxonomy_args=(scripts/run_test_taxonomy_profile.cjs --profile "${taxonomy_profile}" --touched-only --list)
while IFS= read -r path; do
  [[ -z "${path}" ]] && continue
  taxonomy_args+=(--changed-file "${path}")
done <<< "${changed_files}"

taxonomy_commands=()
while IFS= read -r command; do
  [[ -z "${command}" ]] && continue
  taxonomy_commands+=("${command}")
done < <(node "${taxonomy_args[@]}")

if [[ "${#taxonomy_commands[@]}" -eq 0 ]]; then
  echo "no targeted mapping hit; running minimal taxonomy gate"
  run_fast_gate
  exit 0
fi

echo "taxonomy-selected commands:"
printf '%s\n' "${taxonomy_commands[@]}" | sed 's/^/  - /'

for command in "${taxonomy_commands[@]}"; do
  run bash -lc "${command}"
done
