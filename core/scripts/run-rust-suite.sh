#!/usr/bin/env bash
set -euo pipefail

suite="${1:-}"
if [[ -z "${suite}" ]]; then
  echo "usage: $0 {required_integration|cross_platform|soak|load|anomaly|infra_opt_in}" >&2
  exit 2
fi

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

case "${suite}" in
  required_integration)
    run_rust test -q -p ctx-core -p ctx-providers -p ctx-http -p ctx-mcp -p ctx-lsp -p ctx-store
    ;;
  cross_platform)
    run_rust test -q -p ctx-http --test terminal_ws_reconnect
    run_rust test -q -p ctx-http --test workspace_active_snapshot_http
    run_rust test -q -p ctx-http --test workspace_stream_no_gaps_under_activity
    run_rust test -q -p ctx-http --test worktree_archive_http
    ;;
  soak)
    run_rust test -p ctx-http --test workspace_stream_stress_active_heads_lag -- --ignored --nocapture --test-threads=1
    if [[ "$(uname -s)" == "Linux" ]]; then
      run_rust test -p ctx-http --test memory_leak_e2e -- --ignored --nocapture --test-threads=1
    else
      echo "skipping memory_leak_e2e: linux-only"
    fi
    ;;
  load)
    run "${script_dir}/load_smoke.sh"
    ;;
  anomaly)
    run "${script_dir}/run-anomaly-suite.sh"
    ;;
  infra_opt_in)
    run pnpm test:providers:e2e
    run pnpm test:providers:e2e:runner
    ;;
  *)
    echo "usage: $0 {required_integration|cross_platform|soak|load|anomaly|infra_opt_in}" >&2
    exit 2
    ;;
esac
