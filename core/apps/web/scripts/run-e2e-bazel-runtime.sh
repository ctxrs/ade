#!/usr/bin/env bash
set -euo pipefail

resolve_script() {
  local candidate=""
  local runfiles_from_manifest=""
  if [[ -n "${RUNFILES_MANIFEST_FILE:-}" ]]; then
    runfiles_from_manifest="${RUNFILES_MANIFEST_FILE%.runfiles_manifest}.runfiles"
  fi
  for candidate in \
    "${TEST_SRCDIR:-}/${TEST_WORKSPACE:-}/core/apps/web/scripts/run-e2e-bazel-runtime.mjs" \
    "${RUNFILES_DIR:-}/${TEST_WORKSPACE:-}/core/apps/web/scripts/run-e2e-bazel-runtime.mjs" \
    "${RUNFILES_DIR:-}/_main/core/apps/web/scripts/run-e2e-bazel-runtime.mjs" \
    "${runfiles_from_manifest}/${TEST_WORKSPACE:-_main}/core/apps/web/scripts/run-e2e-bazel-runtime.mjs" \
    "${runfiles_from_manifest}/_main/core/apps/web/scripts/run-e2e-bazel-runtime.mjs" \
    "$(cd "$(dirname "$0")" && pwd -P)/$(basename "$0").runfiles/${TEST_WORKSPACE:-_main}/core/apps/web/scripts/run-e2e-bazel-runtime.mjs" \
    "$(cd "$(dirname "$0")" && pwd -P)/$(basename "$0").runfiles/_main/core/apps/web/scripts/run-e2e-bazel-runtime.mjs" \
    "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)/$(basename "${BASH_SOURCE[0]}").runfiles/${TEST_WORKSPACE:-_main}/core/apps/web/scripts/run-e2e-bazel-runtime.mjs" \
    "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)/$(basename "${BASH_SOURCE[0]}").runfiles/_main/core/apps/web/scripts/run-e2e-bazel-runtime.mjs" \
    "${BUILD_WORKSPACE_DIRECTORY:-}/core/apps/web/scripts/run-e2e-bazel-runtime.mjs" \
    "$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)/run-e2e-bazel-runtime.mjs"
  do
    if [[ -n "${candidate}" && -f "${candidate}" ]]; then
      printf '%s\n' "${candidate}"
      return 0
    fi
  done
  return 1
}

resolve_node() {
  local candidate=""
  for candidate in \
    "${NODE:-}" \
    "$(command -v node 2>/dev/null || true)" \
    "${HOME:-}/.local/node"/*/bin/node \
    "/var/lib/buildkite-agent/.local/node"/*/bin/node \
    "/opt/homebrew/bin/node" \
    "/usr/local/bin/node" \
    "/usr/bin/node" \
    "/bin/node"
  do
    if [[ -n "${candidate}" && -x "${candidate}" ]]; then
      printf '%s\n' "${candidate}"
      return 0
    fi
  done
  return 1
}

NODE_BIN="$(resolve_node || true)"
if [[ -z "${NODE_BIN}" ]]; then
  echo "failed to locate node for Bazel web E2E runtime" >&2
  exit 127
fi

SCRIPT="$(resolve_script || true)"
if [[ -z "${SCRIPT}" ]]; then
  echo "failed to locate run-e2e-bazel-runtime.mjs in Bazel runfiles" >&2
  exit 1
fi

if [[ "${SCRIPT}" == *".runfiles/"* ]]; then
  RUNFILES_ROOT="${SCRIPT%%.runfiles/*}.runfiles"
  RUNFILES_REST="${SCRIPT#"${RUNFILES_ROOT}/"}"
  RUNFILES_WORKSPACE="${RUNFILES_REST%%/*}"
  export RUNFILES_DIR="${RUNFILES_DIR:-${RUNFILES_ROOT}}"
  export TEST_WORKSPACE="${TEST_WORKSPACE:-${RUNFILES_WORKSPACE}}"
fi

exec "${NODE_BIN}" "${SCRIPT}" "$@"
