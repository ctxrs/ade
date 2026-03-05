#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

default_output="$(make -n -C "$ROOT/core" dev 2>&1)"
explicit_output="$(make -n -C "$ROOT/core" dev-codex-crp-local 2>&1)"

if [[ "$default_output" != *"pnpm turbo run crp:claude "* ]]; then
  echo "error: default make dev no longer prints the claude build step" >&2
  exit 1
fi
if [[ "$default_output" == *"crp:codex"* ]]; then
  echo "error: default make dev still includes local codex build" >&2
  exit 1
fi
if [[ "$default_output" == *"CTX_INSTALL_LOCAL_CODEX_CRP=\"1\""* || "$default_output" == *"CTX_INSTALL_LOCAL_CODEX_CRP=1"* ]]; then
  echo "error: default make dev still opts into local codex install" >&2
  exit 1
fi

if [[ "$explicit_output" != *"pnpm turbo run crp:claude crp:codex "* ]]; then
  echo "error: explicit make dev-codex-crp-local is missing the local codex build" >&2
  exit 1
fi
if [[ "$explicit_output" != *"CTX_INSTALL_LOCAL_CODEX_CRP=\"1\""* && "$explicit_output" != *"CTX_INSTALL_LOCAL_CODEX_CRP=1"* ]]; then
  echo "error: explicit make dev-codex-crp-local is missing the local codex install opt-in" >&2
  exit 1
fi

echo "ok: make dev uses published codex by default and local codex only via explicit target"
