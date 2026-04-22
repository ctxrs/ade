#!/usr/bin/env bash
set -euo pipefail

repo_root="${BUILD_WORKSPACE_DIRECTORY:-}"
if [[ -z "${repo_root}" || ! -f "${repo_root}/core/package.json" ]]; then
  echo "BUILD_WORKSPACE_DIRECTORY must point at the ctx repository root" >&2
  exit 1
fi

cd "${repo_root}/core"
exec pnpm rust:bazel-deps:check
