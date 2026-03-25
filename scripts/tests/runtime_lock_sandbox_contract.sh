#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
LOCK_JSON="$ROOT/core/apps/desktop/src-tauri/bundles/runtime_lock.v2.json"

python_cmd="python3"
if ! command -v "$python_cmd" >/dev/null 2>&1; then
  if command -v python >/dev/null 2>&1; then
    python_cmd="python"
  else
    echo "error: python3 (or python) is required for this regression test" >&2
    exit 2
  fi
fi

"$python_cmd" - "$LOCK_JSON" <<'PY'
import json
import sys
from pathlib import Path

lock_path = Path(sys.argv[1])
lock = json.loads(lock_path.read_text(encoding="utf-8"))

components = lock.get("components") or []
if not isinstance(components, list):
    raise SystemExit("error: runtime lock components must be an array")

required_runtime_ids = set((lock.get("required") or {}).get("runtime_ids") or [])
errors: list[str] = []

if "avf-linux-guest" not in required_runtime_ids:
    errors.append("required.runtime_ids must include avf-linux-guest")

if any(
    isinstance(component, dict)
    and component.get("kind") == "runtime"
    and component.get("id") == "podman"
    for component in components
):
    errors.append("runtime lock should not include runtime/podman components")

if any(
    isinstance(component, dict)
    and component.get("kind") == "machine_cache"
    and component.get("id") == "podman-machine"
    for component in components
):
    errors.append("runtime lock should not include machine_cache/podman-machine components")

if errors:
    for error in errors:
        print(f"error: {error}", file=sys.stderr)
    raise SystemExit(1)

print("ok: runtime lock matches the sandbox contract (avf guest required, no podman artifacts)")
PY
