#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
core_root="$(cd "${script_dir}/.." && pwd)"
cd "${core_root}"

export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$HOME/.cache/cargo/ctx-monorepo/$(basename "$(git rev-parse --git-dir)")}"

run() {
  echo "+ $*"
  "$@"
}

providers_corpus="crates/ctx-providers/tests/corpus/acp"
mcp_corpus="crates/ctx-mcp/tests/corpus/tools"

if [[ -d "${providers_corpus}" ]]; then
  find "${providers_corpus}" -type f | sort >/tmp/ctx-fuzz-regression-providers-files.txt
  if [[ -s /tmp/ctx-fuzz-regression-providers-files.txt ]]; then
    echo "providers corpus files: $(wc -l </tmp/ctx-fuzz-regression-providers-files.txt)"
  fi
fi

if [[ -d "${mcp_corpus}" ]]; then
  find "${mcp_corpus}" -type f | sort >/tmp/ctx-fuzz-regression-mcp-files.txt
  if [[ -s /tmp/ctx-fuzz-regression-mcp-files.txt ]]; then
    echo "mcp corpus files: $(wc -l </tmp/ctx-fuzz-regression-mcp-files.txt)"
  fi
fi

run cargo test -p ctx-providers --features fuzz_tests
run cargo test -p ctx-mcp --features fuzz_tests
