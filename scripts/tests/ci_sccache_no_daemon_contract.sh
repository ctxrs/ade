#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$repo_root"

workflow_files=(
  "scripts/buildbuddy/run_provider_deps_stage.sh"
  "scripts/buildbuddy/run_codex_crp_stage.sh"
)

for workflow in "${workflow_files[@]}"; do
  wrapper_count="$( (rg -F 'export RUSTC_WRAPPER=sccache' "$workflow" || true) | wc -l | tr -d ' ')"
  no_daemon_count="$( (rg -F 'export SCCACHE_NO_DAEMON=1' "$workflow" || true) | wc -l | tr -d ' ')"
  if [[ "$wrapper_count" != "$no_daemon_count" ]]; then
    echo "sccache no-daemon contract failed for $workflow: wrapper_count=$wrapper_count no_daemon_count=$no_daemon_count"
    exit 1
  fi
done

release_disable_count="$( (rg -F 'CTX_DISABLE_SCCACHE=1' "scripts/buildbuddy/run_release.sh" || true) | wc -l | tr -d ' ')"
if [[ "$release_disable_count" -eq 0 ]]; then
  echo "BuildBuddy release must explicitly disable script-level sccache auto-detection"
  exit 1
fi

echo "ci_sccache_no_daemon_contract: OK"
