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

tmp_root="$(mktemp -d /tmp/ctx-bundle-opencode-managed.XXXXXX)"
trap 'rm -rf "$tmp_root"' EXIT

bundle_dir="$tmp_root/bundle"
CTX_BUNDLE_DIR="$bundle_dir" \
CTX_BUNDLE_ONLY_PROVIDERS="opencode" \
CTX_BUNDLE_SKIP_RUNTIMES=1 \
CTX_BUNDLE_SKIP_IMAGES=1 \
CTX_BUNDLE_INCLUDE_BRIDGE=0 \
CTX_BUNDLE_LOCAL_ADAPTERS=off \
CTX_BUNDLE_BUILD_LOCAL_ADAPTERS=0 \
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
if entry.get("id") != "opencode":
    print(f"expected opencode provider entry, got {entry.get('id')!r}", file=sys.stderr)
    raise SystemExit(1)

if str(entry.get("version", "")) != "1.2.14":
    print(f"expected pinned opencode version 1.2.14, got {entry.get('version')!r}", file=sys.stderr)
    raise SystemExit(1)

command_rel = str(entry.get("command", ""))
if command_rel.endswith(".sh"):
    print(f"opencode command unexpectedly points to shim: {command_rel}", file=sys.stderr)
    raise SystemExit(1)
if "/opencode" not in command_rel:
    print(f"opencode command path is unexpected: {command_rel}", file=sys.stderr)
    raise SystemExit(1)

cmd_path = manifest_path.parent / command_rel
if not cmd_path.is_file():
    print(f"opencode command binary missing: {cmd_path}", file=sys.stderr)
    raise SystemExit(1)

args = entry.get("args") or []
if args != ["acp"]:
    print(f"expected opencode args ['acp'], got {args!r}", file=sys.stderr)
    raise SystemExit(1)

shim_path = manifest_path.parent / "providers" / "opencode" / entry.get("os", "") / entry.get("arch", "") / "opencode-acp.sh"
if shim_path.exists():
    print(f"unexpected legacy opencode shim present: {shim_path}", file=sys.stderr)
    raise SystemExit(1)
PY
then
  echo "error: OpenCode managed bundle validation failed" >&2
  exit 1
fi

echo "ok: ensure_bundled_harnesses bundles OpenCode from managed archive payload"
