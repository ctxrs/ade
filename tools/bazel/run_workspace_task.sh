#!/usr/bin/env bash
set -euo pipefail

if [[ $# -lt 3 ]]; then
  echo "usage: $0 <workspace-relative-dir> <command> <args...>" >&2
  exit 64
fi

resolve_repo_root() {
  local candidate
  for candidate in \
    "${RUNFILES_DIR:-}/_main" \
    "${RUNFILES_DIR:-}/${TEST_WORKSPACE:-}" \
    "${BUILD_WORKSPACE_DIRECTORY:-}"
  do
    if [[ -n "${candidate}" && -f "${candidate}/core/package.json" ]]; then
      printf '%s\n' "${candidate}"
      return 0
    fi
  done
  return 1
}

link_real_workspace_dir() {
  local real_root="$1"
  local temp_root="$2"
  local rel_path="$3"
  if [[ ! -e "${real_root}/${rel_path}" ]]; then
    return 0
  fi
  mkdir -p "$(dirname "${temp_root}/${rel_path}")"
  ln -s "${real_root}/${rel_path}" "${temp_root}/${rel_path}"
}

RUNFILES_REPO_ROOT="$(resolve_repo_root)"
if [[ -z "${RUNFILES_REPO_ROOT}" ]]; then
  echo "failed to locate Bazel runfiles repo root" >&2
  exit 1
fi

REAL_WORKSPACE_ROOT="${BUILD_WORKSPACE_DIRECTORY:-${RUNFILES_REPO_ROOT}}"
TMP_WORKSPACE="$(mktemp -d "${TMPDIR:-/tmp}/ctx-bazel-workspace.XXXXXX")"
trap 'rm -rf "${TMP_WORKSPACE}"' EXIT

RSYNC_EXCLUDES=(
  "--exclude=.git"
  "--exclude=.ctx"
  "--exclude=node_modules"
  "--exclude=.turbo"
  "--exclude=bazel-bin"
  "--exclude=bazel-out"
  "--exclude=bazel-testlogs"
  "--exclude=core/node_modules"
  "--exclude=core/apps/web/node_modules"
  "--exclude=core/target"
  "--exclude=core/.turbo"
  "--exclude=core/apps/web/dist"
)

rsync -a "${RSYNC_EXCLUDES[@]}" "${RUNFILES_REPO_ROOT}/" "${TMP_WORKSPACE}/"
link_real_workspace_dir "${REAL_WORKSPACE_ROOT}" "${TMP_WORKSPACE}" "core/node_modules"
link_real_workspace_dir "${REAL_WORKSPACE_ROOT}" "${TMP_WORKSPACE}" "core/apps/web/node_modules"

REL_DIR="$1"
shift

cd "${TMP_WORKSPACE}/${REL_DIR}"
exec "$@"
