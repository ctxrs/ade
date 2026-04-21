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

usage() {
  echo "usage: $0 {all|ctx-http-fault-matrix|ctx-http-hot-endpoints-no-db|ctx-store-fault-injection}" >&2
}

lane="${1:-all}"
if [[ $# -gt 1 ]]; then
  usage
  exit 2
fi

case "$lane" in
  all)
    run_rust test -p ctx-http --features fault_injection --test fault_matrix
    run_rust test -p ctx-http --features fault_injection --test hot_endpoints_no_db
    run_rust test -p ctx-store --features fault_injection
    ;;
  ctx-http-fault-matrix)
    run_rust test -p ctx-http --features fault_injection --test fault_matrix
    ;;
  ctx-http-hot-endpoints-no-db)
    run_rust test -p ctx-http --features fault_injection --test hot_endpoints_no_db
    ;;
  ctx-store-fault-injection)
    run_rust test -p ctx-store --features fault_injection
    ;;
  *)
    usage
    exit 2
    ;;
esac
