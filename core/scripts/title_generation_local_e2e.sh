#!/usr/bin/env bash
set -euo pipefail

export CTX_E2E_TITLE_GENERATION_LOCAL=1

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "${script_dir}/.." && pwd)"

cd "${repo_root}"
cargo test -p ctx-http --test title_generation_local_e2e -- --ignored --nocapture --test-threads=1
