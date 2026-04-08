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
report_corpus "mcp" "${mcp_corpus}" /tmp/ctx-fuzz-regression-mcp-files.txt
report_corpus "workspace" "${workspace_corpus}" /tmp/ctx-fuzz-regression-workspace-files.txt
report_corpus "updates" "${updates_corpus}" /tmp/ctx-fuzz-regression-updates-files.txt
report_corpus "desktop ipc" "${desktop_ipc_corpus}" /tmp/ctx-fuzz-regression-desktop-ipc-files.txt

run cargo test -p ctx-providers --features fuzz_tests
run cargo test -p ctx-mcp --features fuzz_tests
run cargo test -p ctx-core --test workspace_payload_corpus
run cargo test -p ctx-http --test release_manifest_corpus
run pnpm -C apps/web exec vitest run src/utils/desktop.corpus.test.ts --silent --reporter=dot
