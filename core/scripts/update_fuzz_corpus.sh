#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
usage: update_fuzz_corpus.sh <providers|mcp|workspace|updates|desktop-ipc> <source_dir>

Promote minimized reproducer files into deterministic fuzz regression corpus.

Targets:
  providers   -> crates/ctx-providers/tests/corpus/acp
  mcp         -> crates/ctx-mcp/tests/corpus/tools
  workspace   -> crates/ctx-core/tests/corpus/workspace_payloads
  updates     -> crates/ctx-http/tests/corpus/release_manifests
  desktop-ipc -> apps/web/src/utils/testdata/desktop-ipc
EOF
}

if [[ $# -ne 2 ]]; then
  usage >&2
  exit 2
fi

suite="$1"
source_dir="$2"

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
core_root="$(cd "${script_dir}/.." && pwd)"
cd "${core_root}"

if [[ ! -d "${source_dir}" ]]; then
  echo "error: source directory not found: ${source_dir}" >&2
  exit 1
fi

case "${suite}" in
  providers)
    target_dir="crates/ctx-providers/tests/corpus/acp"
    ;;
  mcp)
    target_dir="crates/ctx-mcp/tests/corpus/tools"
    ;;
  workspace)
    target_dir="crates/ctx-core/tests/corpus/workspace_payloads"
    ;;
  updates)
    target_dir="crates/ctx-http/tests/corpus/release_manifests"
    ;;
  desktop-ipc)
    target_dir="apps/web/src/utils/testdata/desktop-ipc"
    ;;
  *)
    echo "error: unknown suite '${suite}'" >&2
    usage >&2
    exit 2
    ;;
esac

mkdir -p "${target_dir}"

added=0
total=0

while IFS= read -r -d '' file; do
  total=$((total + 1))
  sha="$(shasum -a 256 "${file}" | awk '{print $1}')"
  ext=""
  base="$(basename "${file}")"
  if [[ "${base}" == *.* ]]; then
    ext=".${base##*.}"
  fi
  dest="${target_dir}/${sha}${ext}"
  if [[ -f "${dest}" ]]; then
    continue
  fi
  cp "${file}" "${dest}"
  added=$((added + 1))
done < <(find "${source_dir}" -type f -print0 | sort -z)

echo "corpus update complete: suite=${suite} total=${total} added=${added} target=${target_dir}"
