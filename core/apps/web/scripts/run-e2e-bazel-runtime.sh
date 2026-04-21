#!/usr/bin/env bash
set -euo pipefail

resolve_script() {
  local candidate=""
  for candidate in \
    "${BUILD_WORKSPACE_DIRECTORY:-}/core/apps/web/scripts/run-e2e-bazel-runtime.mjs" \
    "${TEST_SRCDIR:-}/${TEST_WORKSPACE:-}/core/apps/web/scripts/run-e2e-bazel-runtime.mjs" \
    "${RUNFILES_DIR:-}/${TEST_WORKSPACE:-}/core/apps/web/scripts/run-e2e-bazel-runtime.mjs" \
    "${RUNFILES_DIR:-}/_main/core/apps/web/scripts/run-e2e-bazel-runtime.mjs" \
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

exec "${NODE_BIN}" "${SCRIPT}" "$@"
