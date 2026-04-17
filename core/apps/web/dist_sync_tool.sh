#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "usage: $0 <output-dir>" >&2
  exit 64
fi

OUTPUT_DIR="$1"
if [[ -z "${BUILD_WORKSPACE_DIRECTORY:-}" ]]; then
  echo "error: BUILD_WORKSPACE_DIRECTORY is required" >&2
  exit 1
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

REAL_WORKSPACE_ROOT="${BUILD_WORKSPACE_DIRECTORY}"
TMP_WORKSPACE="$(mktemp -d "${TMPDIR:-/tmp}/ctx-bazel-web-dist.XXXXXX")"
trap 'rm -rf "${TMP_WORKSPACE}"' EXIT

TAR_EXCLUDES=(
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
  "--exclude=core/apps/desktop/src-tauri/bin"
  "--exclude=core/apps/desktop/src-tauri/bundles"
  "--exclude=core/apps/web/playwright-report"
  "--exclude=core/apps/web/test-results"
  "--exclude=core/apps/web/e2e/playwright-report"
  "--exclude=core/apps/web/e2e/test-results"
)

mkdir -p "${TMP_WORKSPACE}"
if [[ "${OSTYPE:-}" == darwin* ]]; then
  # AppleDouble and xattr propagation can stall large workspace mirrors on macOS.
  COPYFILE_DISABLE=1 COPY_EXTENDED_ATTRIBUTES_DISABLE=1 \
    tar -C "${RUNFILES_REPO_ROOT}" "${TAR_EXCLUDES[@]}" -cf - . |
    COPYFILE_DISABLE=1 COPY_EXTENDED_ATTRIBUTES_DISABLE=1 \
      tar -C "${TMP_WORKSPACE}" -xf -
else
  tar -C "${RUNFILES_REPO_ROOT}" "${TAR_EXCLUDES[@]}" -cf - . |
    tar -C "${TMP_WORKSPACE}" -xf -
fi
link_real_workspace_dir "${REAL_WORKSPACE_ROOT}" "${TMP_WORKSPACE}" "core/node_modules"
link_real_workspace_dir "${REAL_WORKSPACE_ROOT}" "${TMP_WORKSPACE}" "core/apps/web/node_modules"

VITE_BIN="${TMP_WORKSPACE}/core/apps/web/node_modules/.bin/vite"
if [[ ! -x "${VITE_BIN}" ]]; then
  echo "error: expected web-local vite at ${VITE_BIN}" >&2
  exit 1
fi

cd "${TMP_WORKSPACE}/core/apps/web"
"${VITE_BIN}" build

if [[ ! -d dist ]]; then
  echo "error: expected apps/web/dist after pnpm build" >&2
  exit 1
fi

rm -rf "${OUTPUT_DIR}"
mkdir -p "$(dirname "${OUTPUT_DIR}")"
cp -R dist "${OUTPUT_DIR}"
