#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

eval "$(node scripts/print_ctx_cache_env.cjs --mode workspace --format shell --mkdir)"

node scripts/run_bazel_pilot.cjs test \
  //core/crates/ctx-http:updates_failure_safety_manifest_parse \
  //core/crates/ctx-http:updates_failure_safety_checksum_mismatch \
  //core/crates/ctx-http:updates_failure_safety_missing_artifact \
  //core/crates/ctx-http:updates_failure_safety_interrupted_transfer
