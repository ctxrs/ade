#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
core_root="$(cd "${script_dir}/.." && pwd)"
cd "${core_root}"
eval "$(node scripts/print_ctx_cache_env.cjs --mode workspace --format shell --mkdir)"

run() {
  echo "+ $*"
  "$@"
}

run_rust() {
  echo "+ cargo $*"
  node "${script_dir}/run_with_ctx_cache_env.cjs" --mode workspace --cwd "${core_root}" -- cargo "$@"
}

providers_corpus="crates/ctx-providers/tests/corpus/acp"
mcp_corpus="crates/ctx-mcp/tests/corpus/tools"
workspace_corpus="crates/ctx-core/tests/corpus/workspace_payloads"
updates_corpus="crates/ctx-http/tests/corpus/release_manifests"
desktop_ipc_corpus="apps/web/src/utils/testdata/desktop-ipc"

report_corpus() {
  local label="$1"
  local dir="$2"
  local tmp_file="$3"
  if [[ ! -d "${dir}" ]]; then
    return
  fi
  find "${dir}" -type f | sort >"${tmp_file}"
  if [[ -s "${tmp_file}" ]]; then
    echo "${label} corpus files: $(wc -l <"${tmp_file}")"
  fi
}

report_corpus "providers" "${providers_corpus}" /tmp/ctx-fuzz-regression-providers-files.txt

run_suite() {
  local lane="$1"
  case "$lane" in
    providers)
      report_corpus "providers" "${providers_corpus}" /tmp/ctx-fuzz-regression-providers-files.txt
      run_rust test -p ctx-providers --features fuzz_tests
      ;;
    mcp)
      report_corpus "mcp" "${mcp_corpus}" /tmp/ctx-fuzz-regression-mcp-files.txt
      run_rust test -p ctx-mcp --features fuzz_tests
      ;;
    workspace-payloads)
      report_corpus "workspace" "${workspace_corpus}" /tmp/ctx-fuzz-regression-workspace-files.txt
      run_rust test -p ctx-core --test workspace_payload_corpus
      ;;
    release-manifests)
      report_corpus "updates" "${updates_corpus}" /tmp/ctx-fuzz-regression-updates-files.txt
      run_rust test -p ctx-http --test release_manifest_corpus
      ;;
    desktop-ipc)
      report_corpus "desktop ipc" "${desktop_ipc_corpus}" /tmp/ctx-fuzz-regression-desktop-ipc-files.txt
      run pnpm -C apps/web exec vitest run src/utils/desktop.corpus.test.ts --silent --reporter=dot
      ;;
    *)
      echo "error: unsupported fuzz regression lane '$lane'" >&2
      echo "usage: $0 {all|providers|mcp|workspace-payloads|release-manifests|desktop-ipc}" >&2
      exit 2
      ;;
  esac
}

lane="${1:-all}"
if [[ $# -gt 1 ]]; then
  echo "usage: $0 {all|providers|mcp|workspace-payloads|release-manifests|desktop-ipc}" >&2
  exit 2
fi

case "$lane" in
  all)
    run_suite providers
    run_suite mcp
    run_suite workspace-payloads
    run_suite release-manifests
    run_suite desktop-ipc
    ;;
  *)
    run_suite "$lane"
    ;;
esac
