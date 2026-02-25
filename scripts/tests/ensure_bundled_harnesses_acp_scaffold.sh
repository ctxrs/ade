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

tmp_root="$(mktemp -d /tmp/ctx-bundle-managed-acp.XXXXXX)"
trap 'rm -rf "$tmp_root"' EXIT

bundle_dir="$tmp_root/bundle"
CTX_BUNDLE_DIR="$bundle_dir" \
CTX_BUNDLE_ONLY_PROVIDERS="qwen,mistral,kimi,cline,auggie,cagent" \
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
providers_by_id = {str(p.get("id", "")): p for p in providers}

expected = {
    "qwen",
    "mistral",
    "kimi",
    "cline",
    "auggie",
    "cagent",
}
missing = sorted(expected - set(providers_by_id.keys()))
if missing:
    print("missing managed provider entries:", ", ".join(missing), file=sys.stderr)
    raise SystemExit(1)

for provider_id in sorted(expected):
    entry = providers_by_id[provider_id]
    command_rel = str(entry.get("command", ""))
    if "acp-shims" in command_rel or command_rel.endswith(".sh"):
        print(f"provider {provider_id} still points to shim path: {command_rel}", file=sys.stderr)
        raise SystemExit(1)

# npm-backed providers should execute with bundled node + managed package entrypoint args.
def assert_npm_entry(provider_id: str, package_snippet: str, extra_arg: str | None):
    entry = providers_by_id[provider_id]
    command_rel = str(entry.get("command", ""))
    if "runtimes/node/" not in command_rel:
        print(f"{provider_id} command is not bundled node runtime: {command_rel}", file=sys.stderr)
        raise SystemExit(1)
    args = entry.get("args") or []
    if not args:
        print(f"{provider_id} missing entrypoint args", file=sys.stderr)
        raise SystemExit(1)
    if package_snippet not in str(args[0]):
        print(f"{provider_id} entrypoint arg missing snippet {package_snippet!r}: {args[0]!r}", file=sys.stderr)
        raise SystemExit(1)
    if extra_arg is not None and extra_arg not in args[1:]:
        print(f"{provider_id} args missing required flag {extra_arg!r}: {args!r}", file=sys.stderr)
        raise SystemExit(1)

assert_npm_entry("qwen", "@qwen-code/qwen-code/cli.js", "--experimental-acp")
assert_npm_entry("cline", "node_modules/cline/dist/cli.mjs", "--acp")
assert_npm_entry("auggie", "@augmentcode/auggie/augment.mjs", "--acp")

# archive-backed cagent should resolve to bundled provider payload path.
cagent_cmd = str(providers_by_id["cagent"].get("command", ""))
if "providers/cagent/" not in cagent_cmd:
    print(f"cagent command is not a bundled provider payload: {cagent_cmd}", file=sys.stderr)
    raise SystemExit(1)

# python-backed providers should resolve to bundled virtualenv entrypoints.
for provider_id, expected_args in [("mistral", []), ("kimi", ["--acp"])]:
    entry = providers_by_id[provider_id]
    command_rel = str(entry.get("command", ""))
    if f"providers/{provider_id}/" not in command_rel or "/venv/" not in command_rel:
        print(f"{provider_id} command is not a bundled virtualenv entrypoint: {command_rel}", file=sys.stderr)
        raise SystemExit(1)
    args = entry.get("args") or []
    if args != expected_args:
        print(f"{provider_id} args mismatch: expected {expected_args!r}, got {args!r}", file=sys.stderr)
        raise SystemExit(1)
PY
then
  echo "error: managed ACP provider validation failed" >&2
  exit 1
fi

echo "ok: ensure_bundled_harnesses bundles managed ACP providers without PATH shims"
