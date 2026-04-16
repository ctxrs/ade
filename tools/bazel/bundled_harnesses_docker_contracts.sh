#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="${BUILD_WORKSPACE_DIRECTORY:-}"
if [[ -z "$ROOT_DIR" || ! -f "$ROOT_DIR/core/package.json" ]]; then
  echo "error: BUILD_WORKSPACE_DIRECTORY must point at the repo root" >&2
  exit 1
fi

cd "$ROOT_DIR"

bash scripts/tests/ensure_bundled_harnesses_docker_only.sh
