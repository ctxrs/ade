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

manifest_path="${1:-}"
if [[ -z "$manifest_path" ]]; then
  echo "usage: $(basename "$0") <bundle-manifest-path>" >&2
  exit 2
fi

if [[ ! -f "$manifest_path" ]]; then
  echo "error: manifest not found: $manifest_path" >&2
  exit 2
fi

"$python_cmd" - "$manifest_path" "$LOCK_JSON" <<'PY'
import json
import sys
from pathlib import Path

manifest_path = Path(sys.argv[1])
lock_path = Path(sys.argv[2])

manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
lock = json.loads(lock_path.read_text(encoding="utf-8"))

providers = manifest.get("providers") or []
providers_by_id = {str(p.get("id", "")): p for p in providers if isinstance(p, dict)}
required = [str(v) for v in (lock.get("required", {}).get("provider_ids") or []) if str(v)]

errors: list[str] = []
forbidden_fragments = ("acp-shims", "/usr/bin/env", "command -v", " which ")

for provider_id in required:
    entry = providers_by_id.get(provider_id)
    if entry is None:
        errors.append(f"missing required provider manifest entry: {provider_id}")
        continue

    command_rel = str(entry.get("command", "")).strip()
    if not command_rel:
        errors.append(f"provider command missing: {provider_id}")
        continue

    if command_rel.endswith(".sh"):
        errors.append(f"provider command uses shim shell script: {provider_id} -> {command_rel}")
        continue

    lower_command = command_rel.lower()
    for fragment in forbidden_fragments:
        if fragment in lower_command:
            errors.append(
                f"provider command contains forbidden fallback fragment {fragment!r}: {provider_id} -> {command_rel}"
            )
            break

    command_path = manifest_path.parent / command_rel
    if not command_path.exists():
        errors.append(f"provider command path missing on disk: {provider_id} -> {command_rel}")

if errors:
    for error in errors:
        print(f"error: {error}", file=sys.stderr)
    raise SystemExit(1)

print(
    f"ok: bundle manifest contains all required providers with bundled non-shim commands ({len(required)})"
)
PY
