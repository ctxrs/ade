#!/usr/bin/env bash
set -euo pipefail

eval "$(node scripts/print_ctx_cache_env.cjs --mode workspace --format shell --mkdir)"

cargo test -q -p ctx-http --test updates_failure_safety_manifest_parse
cargo test -q -p ctx-http --test updates_failure_safety_checksum_mismatch
cargo test -q -p ctx-http --test updates_failure_safety_missing_artifact
cargo test -q -p ctx-http --test updates_failure_safety_interrupted_transfer
