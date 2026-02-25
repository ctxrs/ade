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

targets = [("macos", "aarch64"), ("macos", "x86_64")]
errors: list[str] = []

for os_name, arch in targets:
    entry = next(
        (
            component
            for component in components
            if isinstance(component, dict)
            and component.get("kind") == "runtime"
            and component.get("id") == "podman"
            and component.get("os") == os_name
            and component.get("arch") == arch
        ),
        None,
    )
    if entry is None:
        errors.append(f"missing runtime/podman component for {os_name}/{arch}")
        continue

    if not str(entry.get("version") or "").strip():
        errors.append(f"podman component {os_name}/{arch} missing version")
    if not str(entry.get("bin") or "").strip():
        errors.append(f"podman component {os_name}/{arch} missing bin")

    sources = entry.get("sources") or []
    archive = next(
        (
            src
            for src in sources
            if isinstance(src, dict)
            and str(src.get("source_type") or "").strip() in {"vendor", "ci"}
            and str(src.get("uri") or "").strip().startswith("http")
        ),
        None,
    )
    if archive is None:
        errors.append(f"podman component {os_name}/{arch} missing vendor/ci archive uri")
    else:
        if not str(archive.get("sha256") or "").strip():
            errors.append(f"podman component {os_name}/{arch} missing archive sha256")

    for helper_name in ("gvproxy", "vfkit"):
        helper = ((entry.get("helpers") or {}).get(helper_name) or {})
        if not str(helper.get("uri") or "").strip().startswith("http"):
            errors.append(f"podman component {os_name}/{arch} missing {helper_name} uri")
        if not str(helper.get("sha256") or "").strip():
            errors.append(f"podman component {os_name}/{arch} missing {helper_name} sha256")

if errors:
    for error in errors:
        print(f"error: {error}", file=sys.stderr)
    raise SystemExit(1)

print("ok: runtime lock includes pinned podman sources + checksums for macOS targets")
PY
