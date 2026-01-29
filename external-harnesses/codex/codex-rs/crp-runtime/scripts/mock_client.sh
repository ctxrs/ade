#!/usr/bin/env bash
set -euo pipefail

crp_bin="${1:-codex-crp}"

{
  printf '%s\n' '{"type":"session.open","session_id":"mock-session","config":{"reasoning_trace_enabled":true}}'
  sleep 1
  printf '%s\n' '{"type":"session.prompt","session_id":"mock-session","turn_id":"mock-turn-1","prompt":"Say hello, then run `pwd`."}'
  sleep 1
  printf '%s\n' '{"type":"session.cancel","session_id":"mock-session"}'
} | "$crp_bin"
