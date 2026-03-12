#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CRP_WORKSPACE="${ROOT_DIR}/external-harnesses/codex/codex-rs"
PROFILE="${CTX_CRP_PROFILE:-debug}"

# shellcheck source=lib/codex_crp_build_env.sh
source "${ROOT_DIR}/scripts/lib/codex_crp_build_env.sh"

TARGET_DIR="$(codex_crp_target_dir "${ROOT_DIR}")"
BUILD_JOBS="$(codex_crp_build_jobs)"
DATA_DIR="${CTX_DATA_DIR:-${HOME}/.ctx}"
INSTALL_DIR="${DATA_DIR}/providers/agent-servers/codex-crp/dev"
INSTALL_BIN="${INSTALL_DIR}/codex-crp"
AGENT_CFG="${DATA_DIR}/providers/agent-servers/agent_servers.json"

if [[ ! -d "${CRP_WORKSPACE}" ]]; then
  echo "error: codex-crp workspace not found at ${CRP_WORKSPACE}" >&2
  exit 1
fi

profile_args=()
if [[ "${PROFILE}" == "release" ]]; then
  profile_args+=(--release)
fi

(
  cd "${CRP_WORKSPACE}"
  mkdir -p "${TARGET_DIR}"
  CARGO_TARGET_DIR="${TARGET_DIR}" CARGO_BUILD_JOBS="${BUILD_JOBS}" cargo build -p codex-crp "${profile_args[@]}"
)

if [[ ! -f "${TARGET_DIR}/${PROFILE}/codex-crp" ]]; then
  echo "error: codex-crp binary not found at ${TARGET_DIR}/${PROFILE}/codex-crp" >&2
  exit 1
fi

mkdir -p "${INSTALL_DIR}"
install -m 755 "${TARGET_DIR}/${PROFILE}/codex-crp" "${INSTALL_BIN}"

AGENT_CFG="${AGENT_CFG}" INSTALL_BIN="${INSTALL_BIN}" python3 - <<'PY'
import json
import os
from pathlib import Path

cfg_path = Path(os.environ["AGENT_CFG"])
install_bin = os.environ["INSTALL_BIN"]

payload = {"providers": {}, "managed_installs": {}}
if cfg_path.exists():
    raw = cfg_path.read_text().strip()
    if raw:
        payload = json.loads(raw)

providers = payload.get("providers")
if not isinstance(providers, dict):
    providers = {}
managed = payload.get("managed_installs")
if not isinstance(managed, dict):
    managed = {}

providers["codex"] = {
    "command": install_bin,
    "args": [],
    "dependencies": [],
}

payload["providers"] = providers
payload["managed_installs"] = managed
cfg_path.parent.mkdir(parents=True, exist_ok=True)
cfg_path.write_text(json.dumps(payload, indent=2) + "\n")
PY
