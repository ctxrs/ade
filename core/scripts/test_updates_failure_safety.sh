#!/usr/bin/env bash
set -euo pipefail

CARGO_TARGET_DIR=${CARGO_TARGET_DIR:-$HOME/.cache/cargo/ctx-monorepo/$(basename "$(git rev-parse --git-dir)")}
export CARGO_TARGET_DIR

cargo test -q -p ctx-http --test updates_failure_safety_manifest_parse
cargo test -q -p ctx-http --test updates_failure_safety_checksum_mismatch
cargo test -q -p ctx-http --test updates_failure_safety_missing_artifact
cargo test -q -p ctx-http --test updates_failure_safety_interrupted_transfer
