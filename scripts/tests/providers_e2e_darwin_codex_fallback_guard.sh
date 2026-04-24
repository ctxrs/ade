#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$ROOT/core/scripts/providers_e2e.sh"

if [[ ! -f "$SCRIPT" ]]; then
  echo "missing providers e2e script: $SCRIPT" >&2
  exit 1
fi

require_pattern() {
  local pattern="$1"
  if ! grep -Fq "$pattern" "$SCRIPT"; then
    echo "missing expected pattern in providers_e2e.sh: $pattern" >&2
    exit 1
  fi
}

require_pattern 'matrix_has_archive_target "${matrix_json}" "codex-crp" "linux-aarch64"'
require_pattern 'codex_append_fallback_arch="x86_64"'
require_pattern 'CTX_BUNDLE_ONLY_PROVIDERS="codex-crp"'
require_pattern 'CTX_BUNDLE_ARCH="${codex_append_fallback_arch}"'
require_pattern 'CTX_BUNDLE_LOCAL_ADAPTERS="off"'

echo "ok: darwin codex fallback append guard present in providers_e2e.sh"
