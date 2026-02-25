#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
MATRIX_JSON="$ROOT/core/crates/ctx-http/src/provider_matrix.json"
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

"$python_cmd" - "$MATRIX_JSON" "$LOCK_JSON" <<'PY'
import json
import sys
from pathlib import Path

matrix_path = Path(sys.argv[1])
lock_path = Path(sys.argv[2])

matrix = json.loads(matrix_path.read_text(encoding="utf-8"))
lock = json.loads(lock_path.read_text(encoding="utf-8"))

providers = {str(p.get("id", "")): p for p in (matrix.get("providers") or [])}
required = [str(v) for v in (lock.get("required", {}).get("provider_ids") or []) if str(v)]

errors: list[str] = []

for provider_id in required:
    entry = providers.get(provider_id)
    if not entry:
        errors.append(f"missing provider matrix entry: {provider_id}")
        continue

    managed = entry.get("managed_install")
    if not isinstance(managed, dict):
        errors.append(f"provider missing managed_install: {provider_id}")
        continue

    kind = str(managed.get("kind") or "").strip()
    if not kind:
        errors.append(f"provider managed_install.kind missing: {provider_id}")
        continue

    releases = entry.get("releases") or []
    if not isinstance(releases, list) or not releases:
        errors.append(f"provider releases missing: {provider_id}")
    else:
        if not any(str(r.get("version", "")).strip() for r in releases if isinstance(r, dict)):
            errors.append(f"provider releases have no pinned version values: {provider_id}")

    if kind in {"archive", "python"}:
        if not str(managed.get("version", "")).strip():
            errors.append(f"provider managed_install.version missing: {provider_id}")
    elif kind == "npm":
        if not str(managed.get("package", "")).strip():
            errors.append(f"provider npm package missing: {provider_id}")
        if not str(managed.get("entrypoint", "")).strip():
            errors.append(f"provider npm entrypoint missing: {provider_id}")
    else:
        errors.append(f"provider managed_install.kind unsupported: {provider_id} ({kind})")

if errors:
    for error in errors:
        print(f"error: {error}", file=sys.stderr)
    raise SystemExit(1)

print(f"ok: required providers have managed source + pinned versions ({len(required)})")
PY
