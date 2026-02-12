#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DATA_DIR="${CTX_DATA_DIR:-${HOME}/.ctx}"
PROFILE="${CTX_CRP_PROFILE:-debug}"

CODEX_WORKSPACE="${ROOT_DIR}/external-harnesses/codex/codex-rs"
CODEX_TARGET_DIR="${CTX_CRP_TARGET_DIR:-${CODEX_WORKSPACE}/target}"
CODEX_BIN_SRC="${CODEX_TARGET_DIR}/${PROFILE}/codex-crp"
CODEX_INSTALL_DIR="${DATA_DIR}/providers/agent-servers/codex-crp/dev"
CODEX_INSTALL_BIN="${CODEX_INSTALL_DIR}/codex-crp"

CLAUDE_WORKSPACE="${ROOT_DIR}/external-harnesses/claude-crp"
CLAUDE_BIN="${CLAUDE_WORKSPACE}/bin/claude-crp"
CLAUDE_DIST="${CLAUDE_WORKSPACE}/dist/runtime.js"

AGENT_CFG="${DATA_DIR}/providers/agent-servers/agent_servers.json"

if [[ ! -f "${CODEX_BIN_SRC}" ]]; then
  echo "error: codex-crp binary not found at ${CODEX_BIN_SRC} (run build first)" >&2
  exit 1
fi
if [[ ! -f "${CLAUDE_BIN}" ]]; then
  echo "error: claude-crp entrypoint missing at ${CLAUDE_BIN}" >&2
  exit 1
fi
if [[ ! -f "${CLAUDE_DIST}" ]]; then
  echo "error: claude-crp dist missing at ${CLAUDE_DIST} (run build first)" >&2
  exit 1
fi

mkdir -p "${CODEX_INSTALL_DIR}"
install -m 755 "${CODEX_BIN_SRC}" "${CODEX_INSTALL_BIN}"

AGENT_CFG="${AGENT_CFG}" \
CODEX_BIN="${CODEX_INSTALL_BIN}" \
CLAUDE_BIN="${CLAUDE_BIN}" \
python3 - <<'PY'
import json
import os
from pathlib import Path

cfg_path = Path(os.environ["AGENT_CFG"])
codex_bin = os.environ["CODEX_BIN"]
claude_bin = os.environ["CLAUDE_BIN"]

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
    "command": codex_bin,
    "args": [],
    "dependencies": [],
}
providers["claude-crp"] = {
    "command": claude_bin,
    "args": [],
    "dependencies": [],
}

payload["providers"] = providers
payload["managed_installs"] = managed
cfg_path.parent.mkdir(parents=True, exist_ok=True)
cfg_path.write_text(json.dumps(payload, indent=2) + "\n")
PY
