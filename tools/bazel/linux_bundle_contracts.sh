#!/usr/bin/env bash
set -euo pipefail

resolve_repo_root() {
  local candidate
  for candidate in \
    "${BUILD_WORKSPACE_DIRECTORY:-}" \
    "${RUNFILES_DIR:-}/_main" \
    "${RUNFILES_DIR:-}/${TEST_WORKSPACE:-}"
  do
    if [[ -n "${candidate}" && -f "${candidate}/core/package.json" ]]; then
      printf '%s\n' "${candidate}"
      return 0
    fi
  done
  return 1
}

REPO_ROOT="$(resolve_repo_root)"
if [[ -z "$REPO_ROOT" ]]; then
  echo "error: failed to locate repo root for linux bundle contracts" >&2
  exit 1
fi

platform=""
bundles_dir="core/apps/desktop/src-tauri/bundles"
prune_glibc=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --platform)
      if [[ $# -lt 2 || -z "${2:-}" ]]; then
        echo "error: --platform requires a value" >&2
        exit 2
      fi
      platform="$2"
      shift 2
      ;;
    --bundles-dir)
      if [[ $# -lt 2 || -z "${2:-}" ]]; then
        echo "error: --bundles-dir requires a value" >&2
        exit 2
      fi
      bundles_dir="$2"
      shift 2
      ;;
    --prune-glibc)
      prune_glibc=1
      shift
      ;;
    *)
      echo "error: unsupported linux bundle contracts arg '$1'" >&2
      exit 2
      ;;
  esac
done

cd "$REPO_ROOT"

if [[ "$prune_glibc" == "1" ]]; then
  "$REPO_ROOT/scripts/linux_bundle_prune_glibc.sh" --platform "$platform" --bundles-dir "$bundles_dir"
fi

"$REPO_ROOT/scripts/linux_bundle_gate.sh" --platform "$platform" --bundles-dir "$bundles_dir" --mode both
