#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
core_dir="$root_dir/core"

if [ -z "${GCP_SERVICE_ACCOUNT_JSON:-}" ] && [ -z "${GOOGLE_APPLICATION_CREDENTIALS:-}" ]; then
  echo "[e2e] missing GCP_SERVICE_ACCOUNT_JSON or GOOGLE_APPLICATION_CREDENTIALS" >&2
  exit 1
fi

python_bin="$(command -v python3 || command -v python || true)"
if [ -z "$python_bin" ]; then
  echo "[e2e] python is required to parse GCP service account JSON" >&2
  exit 1
fi

if [ -n "${GCP_SERVICE_ACCOUNT_JSON:-}" ]; then
  sa_file="$(mktemp)"
  printf '%s' "$GCP_SERVICE_ACCOUNT_JSON" > "$sa_file"
  export GOOGLE_APPLICATION_CREDENTIALS="$sa_file"
  trap 'rm -f "$sa_file"' EXIT
fi

if [ -z "${GCP_PROJECT_ID:-}" ]; then
  export GCP_PROJECT_ID="$("$python_bin" - <<'PY'
import json, os
data=json.loads(os.environ["GCP_SERVICE_ACCOUNT_JSON"])
print(data.get("project_id",""))
PY
)"
fi
if [ -z "${GCP_PROJECT_ID:-}" ]; then
  echo "[e2e] missing GCP_PROJECT_ID" >&2
  exit 1
fi

if [ -z "${GCP_SERVICE_ACCOUNT:-}" ] && [ -z "${GCP_SERVICE_ACCOUNT_EMAIL:-}" ]; then
  export GCP_SERVICE_ACCOUNT_EMAIL="$("$python_bin" - <<'PY'
import json, os
data=json.loads(os.environ["GCP_SERVICE_ACCOUNT_JSON"])
print(data.get("client_email",""))
PY
)"
fi

export GCP_ZONE="${GCP_ZONE:-us-central1-a}"
export GCP_MACHINE_TYPE="${GCP_MACHINE_TYPE:-e2-standard-2}"
export GCP_IMAGE="${GCP_IMAGE:-projects/debian-cloud/global/images/family/debian-12}"
export GCP_DISK_SIZE_GB="${GCP_DISK_SIZE_GB:-100}"
export GCP_DISK_TYPE="${GCP_DISK_TYPE:-pd-ssd}"

export CTX_E2E_TIER=3
export CTX_E2E_PROVIDER_ID="${CTX_E2E_PROVIDER_ID:-codex}"
export CTX_E2E_MODEL_ID="${CTX_E2E_MODEL_ID:-gpt-5.2-codex}"
export CTX_E2E_KEEP_RESOURCES="${CTX_E2E_KEEP_RESOURCES:-0}"

repo_dir="${CTX_E2E_WORKSPACE_ROOT:-/tmp/ctx-e2e-public-repo}"
if [ ! -d "$repo_dir/.git" ]; then
  rm -rf "$repo_dir"
  git clone https://github.com/octocat/Hello-World.git "$repo_dir"
fi
export CTX_E2E_WORKSPACE_ROOT="$repo_dir"

cd "$core_dir"

CARGO_TARGET_DIR=target cargo build -p ctx-worker-gateway -p ctx-worker-shim

export CTX_WORKER_GATEWAY_BIN="$core_dir/target/debug/ctx-worker-gateway"
export CTX_WORKER_SHIM_BIN="$core_dir/target/debug/ctx-worker-shim"

CARGO_TARGET_DIR=target cargo test -p ctx-http --test cloud_gateway_gcp_e2e -- --ignored --nocapture --test-threads=1
