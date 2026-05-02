#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

default_output="$(make -n -C "$ROOT/core" dev 2>&1)"
explicit_output="$(make -n -C "$ROOT/core" dev-codex-crp-local 2>&1)"

if [[ "$default_output" != *"pnpm crp:claude"* ]]; then
  echo "error: default make dev no longer prints the claude build step" >&2
  exit 1
fi
if [[ "$default_output" != *'if [ "0" = "1" ]; then CTX_CRP_PROFILE="debug" pnpm crp:codex; fi'* ]]; then
  echo "error: default make dev no longer guards the local codex build behind the opt-in flag" >&2
  exit 1
fi
if [[ "$default_output" == *"CTX_INSTALL_LOCAL_CODEX_CRP=\"1\""* || "$default_output" == *"CTX_INSTALL_LOCAL_CODEX_CRP=1"* ]]; then
  echo "error: default make dev still opts into local codex install" >&2
  exit 1
fi

if [[ "$explicit_output" != *"pnpm crp:claude"* || "$explicit_output" != *'if [ "1" = "1" ]; then CTX_CRP_PROFILE="debug" pnpm crp:codex; fi'* ]]; then
  echo "error: explicit make dev-codex-crp-local is missing the local codex build" >&2
  exit 1
fi
if [[ "$explicit_output" != *"CTX_INSTALL_LOCAL_CODEX_CRP=\"1\""* && "$explicit_output" != *"CTX_INSTALL_LOCAL_CODEX_CRP=1"* ]]; then
  echo "error: explicit make dev-codex-crp-local is missing the local codex install opt-in" >&2
  exit 1
fi

echo "ok: make dev uses published codex by default and local codex only via explicit target"
