#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="${BUILD_WORKSPACE_DIRECTORY:-}"
if [[ -z "$ROOT_DIR" || ! -f "$ROOT_DIR/core/package.json" ]]; then
  echo "error: BUILD_WORKSPACE_DIRECTORY must point at the repo root" >&2
  exit 1
fi

codex_target=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --codex-target)
      if [[ $# -lt 2 || -z "${2:-}" ]]; then
        echo "error: --codex-target requires a value" >&2
        exit 2
      fi
      codex_target="$2"
      shift 2
      ;;
    *)
      echo "error: unsupported release bundle contracts arg '$1'" >&2
      exit 2
      ;;
  esac
done

cd "$ROOT_DIR"

node core/scripts/bundled_dependency_updates.cjs policy
bash scripts/tests/codex_release_provenance_policy.sh
if [[ -n "$codex_target" ]]; then
  node core/scripts/provider_matrix_archive_artifact_gate.cjs --provider codex --target "$codex_target"
fi
bash scripts/tests/desktop_merge_bundles.sh
bash scripts/tests/ensure_bundled_harnesses_docker_only.sh
