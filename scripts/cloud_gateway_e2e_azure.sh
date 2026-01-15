#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
core_dir="$root_dir/core"

require_env() {
  local name="$1"
  if [ -z "${!name:-}" ]; then
    echo "[e2e] missing ${name}" >&2
    exit 1
  fi
}

require_env AZURE_SUBSCRIPTION_ID
require_env AZURE_RESOURCE_GROUP
require_env AZURE_VNET
require_env AZURE_SUBNET

if [ -z "${AZURE_SSH_PUBLIC_KEY:-}" ] && [ -z "${AZURE_SSH_PUBLIC_KEY_PATH:-}" ]; then
  if [ -n "${AZURE_E2E_SSH_PRIVATE_KEY:-}" ]; then
    if ! command -v ssh-keygen >/dev/null 2>&1; then
      echo "[e2e] ssh-keygen is required to derive AZURE_SSH_PUBLIC_KEY" >&2
      exit 1
    fi
    key_file="$(mktemp)"
    printf '%s' "$AZURE_E2E_SSH_PRIVATE_KEY" > "$key_file"
    chmod 600 "$key_file"
    trap 'rm -f "$key_file"' EXIT
    export AZURE_SSH_PUBLIC_KEY="$(ssh-keygen -y -f "$key_file")"
  else
    echo "[e2e] missing AZURE_SSH_PUBLIC_KEY or AZURE_SSH_PUBLIC_KEY_PATH" >&2
    exit 1
  fi
fi

export AZURE_LOCATION="${AZURE_LOCATION:-eastus}"
export AZURE_VM_SIZE="${AZURE_VM_SIZE:-Standard_D2s_v3}"
export AZURE_IMAGE="${AZURE_IMAGE:-Canonical:0001-com-ubuntu-server-jammy:22_04-lts-gen2:latest}"
export AZURE_DISK_SIZE_GB="${AZURE_DISK_SIZE_GB:-100}"
export AZURE_DISK_SKU="${AZURE_DISK_SKU:-Premium_LRS}"
export AZURE_USE_PUBLIC_IP="${AZURE_USE_PUBLIC_IP:-true}"
export AZURE_DELETE_DISK_ON_PAUSE="${AZURE_DELETE_DISK_ON_PAUSE:-false}"

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

CARGO_TARGET_DIR=target cargo test -p ctx-http --test cloud_gateway_azure_e2e -- --ignored --nocapture --test-threads=1
