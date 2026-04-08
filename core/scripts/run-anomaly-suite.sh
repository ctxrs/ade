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

run cargo test -p ctx-http --features fault_injection --test fault_matrix
run cargo test -p ctx-http --features fault_injection --test hot_endpoints_no_db
run cargo test -p ctx-store --features fault_injection
