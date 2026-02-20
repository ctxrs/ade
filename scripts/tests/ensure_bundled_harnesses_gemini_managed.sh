#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BUNDLE_SCRIPT="$ROOT/scripts/ensure_bundled_harnesses.sh"

if [[ ! -x "$BUNDLE_SCRIPT" ]]; then
  echo "error: missing bundle script: $BUNDLE_SCRIPT" >&2
  exit 2
fi

python_cmd="python3"
if ! command -v "$python_cmd" >/dev/null 2>&1; then
  if command -v python >/dev/null 2>&1; then
    python_cmd="python"
  else
    echo "error: python3 (or python) is required for this regression test" >&2
    exit 2
  fi
fi

tmp_root="$(mktemp -d /tmp/ctx-bundle-gemini-managed.XXXXXX)"
trap 'rm -rf "$tmp_root"' EXIT

bundle_dir="$tmp_root/bundle"
CTX_BUNDLE_DIR="$bundle_dir" \
CTX_BUNDLE_ONLY_PROVIDERS="gemini" \
CTX_BUNDLE_SKIP_IMAGES=1 \
CTX_BUNDLE_INCLUDE_BRIDGE=0 \
CTX_BUNDLE_BUILD_CODEX_CRP=0 \
"$BUNDLE_SCRIPT" >/dev/null

if ! "$python_cmd" - "$bundle_dir/manifest.json" <<'PY'
import json
import sys
from pathlib import Path

manifest_path = Path(sys.argv[1])
data = json.loads(manifest_path.read_text(encoding="utf-8"))
providers = data.get("providers") or []
if len(providers) != 1:
    print(f"expected exactly one provider entry, got {len(providers)}", file=sys.stderr)
    raise SystemExit(1)

entry = providers[0]
if entry.get("id") != "gemini":
    print(f"expected gemini provider entry, got {entry.get('id')!r}", file=sys.stderr)
    raise SystemExit(1)

command_rel = str(entry.get("command", ""))
if "runtimes/node/" not in command_rel:
    print(f"gemini command is not bundled node runtime: {command_rel}", file=sys.stderr)
    raise SystemExit(1)

args = entry.get("args") or []
if len(args) < 2:
    print(f"gemini args are incomplete: {args}", file=sys.stderr)
    raise SystemExit(1)
if "@google/gemini-cli/dist/index.js" not in str(args[0]):
    print(f"gemini entrypoint arg is not managed npm payload: {args[0]}", file=sys.stderr)
    raise SystemExit(1)
if args[1] != "--experimental-acp":
    print(f"gemini second arg should be --experimental-acp, got {args[1]!r}", file=sys.stderr)
    raise SystemExit(1)

shim_path = manifest_path.parent / "providers" / "gemini" / entry.get("os", "") / entry.get("arch", "") / "gemini-acp.sh"
if shim_path.exists():
    print(f"unexpected legacy gemini shim present: {shim_path}", file=sys.stderr)
    raise SystemExit(1)
PY
then
  echo "error: Gemini managed bundle validation failed" >&2
  exit 1
fi

echo "ok: ensure_bundled_harnesses bundles Gemini from managed npm payload"
