#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$repo_root"

workflow_files=(
  ".github/workflows/build-codex-crp-release-artifacts.yml"
  ".github/workflows/publish-provider-deps.yml"
  ".github/workflows/release-preflight.yml"
  ".github/workflows/release-supabase.yml"
)

for workflow in "${workflow_files[@]}"; do
  wrapper_count="$(rg -F 'echo "RUSTC_WRAPPER=sccache"' "$workflow" | wc -l | tr -d ' ')"
  no_daemon_count="$(rg -F 'echo "SCCACHE_NO_DAEMON=1"' "$workflow" | wc -l | tr -d ' ')"
  if [[ "$wrapper_count" != "$no_daemon_count" ]]; then
    echo "sccache no-daemon contract failed for $workflow: wrapper_count=$wrapper_count no_daemon_count=$no_daemon_count"
    exit 1
  fi
done

echo "ci_sccache_no_daemon_contract: OK"
