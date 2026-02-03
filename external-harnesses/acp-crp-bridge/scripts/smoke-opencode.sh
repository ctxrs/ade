#!/usr/bin/env bash
set -euo pipefail

if ! command -v opencode >/dev/null 2>&1; then
  echo "opencode not found in PATH" >&2
  exit 1
fi

ROOT_DIR="${PWD}"

# Example CRP session flow. The bridge keeps running; Ctrl+C to exit.
cat <<JSON | acp-crp-bridge --acp-command "opencode acp" --acp-cwd "${ROOT_DIR}"
{"type":"session.open","session_id":"smoke","config":{"cwd":"${ROOT_DIR}"}}
{"type":"session.prompt","session_id":"smoke","turn_id":"turn-1","prompt":"Say hello in one sentence."}
JSON
