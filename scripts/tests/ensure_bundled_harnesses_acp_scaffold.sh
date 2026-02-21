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

tmp_root="$(mktemp -d /tmp/ctx-bundle-acp-scaffold.XXXXXX)"
trap 'rm -rf "$tmp_root"' EXIT

bundle_dir="$tmp_root/bundle"
CTX_BUNDLE_DIR="$bundle_dir" \
CTX_BUNDLE_ONLY_PROVIDERS="qwen,opencode,mistral,goose,kimi,auggie,continue,openhands,amp,droid,copilot,kiro" \
CTX_BUNDLE_SKIP_RUNTIMES=1 \
CTX_BUNDLE_SKIP_IMAGES=1 \
CTX_BUNDLE_INCLUDE_BRIDGE=0 \
CTX_BUNDLE_LOCAL_ADAPTERS=off \
CTX_BUNDLE_BUILD_LOCAL_ADAPTERS=0 \
CTX_BUNDLE_BUILD_CODEX_CRP=0 \
"$BUNDLE_SCRIPT"

if ! "$python_cmd" - "$bundle_dir/manifest.json" <<'PY'
import json
import sys
from pathlib import Path

manifest_path = Path(sys.argv[1])
data = json.loads(manifest_path.read_text(encoding="utf-8"))
providers = data.get("providers") or []
ids = {str(p.get("id", "")) for p in providers}
providers_by_id = {str(p.get("id", "")): p for p in providers}

expected = {
    "qwen",
    "opencode",
    "mistral",
    "goose",
    "kimi",
    "auggie",
    "continue",
    "openhands",
    "amp",
    "droid",
    "copilot",
    "kiro",
}

missing = sorted(expected - ids)
if missing:
    print("missing ACP scaffold provider entries:", ", ".join(missing), file=sys.stderr)
    raise SystemExit(1)

shim_expectations = {
    "continue": [
        "for candidate in cn continue; do",
        'default_args="acp"',
    ],
    "qwen": [
        "for candidate in qwen qwen-code; do",
        'default_args="--experimental-acp"',
    ],
    "copilot": [
        "for candidate in copilot-cli-acp github-copilot-cli copilot; do",
    ],
}

for provider_id, snippets in shim_expectations.items():
    entry = providers_by_id.get(provider_id)
    if not entry:
        print(f"missing manifest entry for {provider_id}", file=sys.stderr)
        raise SystemExit(1)
    command_rel = str(entry.get("command", ""))
    shim_path = manifest_path.parent / command_rel
    if not shim_path.is_file():
        print(f"missing shim file for {provider_id}: {shim_path}", file=sys.stderr)
        raise SystemExit(1)
    shim_body = shim_path.read_text(encoding="utf-8")
    for snippet in snippets:
        if snippet not in shim_body:
            print(
                f"shim for {provider_id} missing expected snippet: {snippet}",
                file=sys.stderr,
            )
            raise SystemExit(1)
PY
then
  echo "error: ACP scaffold provider validation failed" >&2
  exit 1
fi

echo "ok: ensure_bundled_harnesses scaffolds ACP providers"
