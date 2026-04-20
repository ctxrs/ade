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
bash tools/bazel/codex_provenance_policy.sh
if [[ "${CTX_RELEASE_VERIFY_PUBLISHED_PROVIDER_ARTIFACTS:-0}" == "1" && -n "$codex_target" ]]; then
  bash tools/bazel/codex_archive_artifact_gate.sh --codex-target "$codex_target"
fi
bash scripts/tests/desktop_merge_bundles.sh
