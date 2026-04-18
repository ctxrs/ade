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
      echo "error: unsupported codex archive artifact gate arg '$1'" >&2
      exit 2
      ;;
  esac
done

cd "$ROOT_DIR"

archive_args=(--provider codex)
if [[ -n "$codex_target" ]]; then
  archive_args+=(--target "$codex_target")
fi

node core/scripts/provider_matrix_archive_artifact_gate.cjs "${archive_args[@]}"
