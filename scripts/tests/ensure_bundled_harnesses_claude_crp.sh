#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BUNDLE_SCRIPT="$ROOT/scripts/ensure_bundled_harnesses.sh"

if [[ ! -x "$BUNDLE_SCRIPT" ]]; then
  echo "error: missing bundle script: $BUNDLE_SCRIPT" >&2
  exit 2
fi

for cmd in node npm python3; do
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "error: $cmd is required for this regression test" >&2
    exit 2
  fi
done

tmp_root="$(mktemp -d /tmp/ctx-bundle-claude-crp.XXXXXX)"
trap 'rm -rf "$tmp_root"' EXIT

bundle_dir="$tmp_root/bundle"
claude_ws="$tmp_root/claude-crp"
mkdir -p "$claude_ws/bin" "$claude_ws/dist"

cat > "$claude_ws/package.json" <<'JSON'
{
  "name": "claude-crp",
  "version": "0.0.0-test",
  "private": true,
  "type": "module",
  "dependencies": {}
}
JSON

cat > "$claude_ws/bin/claude-crp" <<'JS'
#!/usr/bin/env node
import { runRuntime } from "../dist/runtime.js";
await runRuntime();
JS
chmod +x "$claude_ws/bin/claude-crp"

cat > "$claude_ws/dist/runtime.js" <<'JS'
export async function runRuntime() {}
JS

node_version="$(python3 - "$ROOT/core/crates/ctx-http/src/installer.rs" <<'PY'
import re
import sys
src = open(sys.argv[1], "r", encoding="utf-8").read()
m = re.search(r'const\s+NODE_VERSION:\s*&str\s*=\s*"([^"]+)"', src)
if not m:
    raise SystemExit(1)
print(m.group(1))
PY
)"
if [[ -z "$node_version" ]]; then
  echo "error: failed to resolve NODE_VERSION" >&2
  exit 2
fi

os_raw="$(uname -s 2>/dev/null || true)"
case "$os_raw" in
  Darwin) os="macos" ;;
  Linux) os="linux" ;;
  *)
    echo "error: unsupported host os for this test: $os_raw" >&2
    exit 2
    ;;
esac

arch_raw="$(uname -m 2>/dev/null || true)"
case "$arch_raw" in
  x86_64|amd64) arch="x86_64" ;;
  aarch64|arm64) arch="aarch64" ;;
  *)
    echo "error: unsupported host arch for this test: $arch_raw" >&2
    exit 2
    ;;
esac

case "${os}/${arch}" in
  macos/x86_64) node_target="darwin-x64" ;;
  macos/aarch64) node_target="darwin-arm64" ;;
  linux/x86_64) node_target="linux-x64" ;;
  linux/aarch64) node_target="linux-arm64" ;;
  *)
    echo "error: unsupported node target for ${os}/${arch}" >&2
    exit 2
    ;;
esac

node_root="$bundle_dir/runtimes/node/${os}/${arch}/node-v${node_version}-${node_target}"
mkdir -p "$node_root/bin" "$node_root/lib/node_modules/npm/bin"

node_bin_src="$(command -v node)"
npm_cli_src="$(node -e 'process.stdout.write(require.resolve("npm/bin/npm-cli.js"))' 2>/dev/null || true)"
if [[ -z "$npm_cli_src" || ! -f "$npm_cli_src" ]]; then
  npm_cli_src="$(npm root -g 2>/dev/null)/npm/bin/npm-cli.js"
fi
if [[ ! -f "$npm_cli_src" ]]; then
  echo "error: failed to resolve npm-cli.js from host npm install" >&2
  exit 2
fi

ln -s "$node_bin_src" "$node_root/bin/node"
ln -s "$npm_cli_src" "$node_root/lib/node_modules/npm/bin/npm-cli.js"

CTX_BUNDLE_DIR="$bundle_dir" \
CTX_BUNDLE_ONLY_PROVIDERS="claude-crp" \
CTX_BUNDLE_SKIP_IMAGES=1 \
CTX_BUNDLE_INCLUDE_BRIDGE=0 \
CTX_BUNDLE_LOCAL_ADAPTERS=off \
CTX_BUNDLE_BUILD_LOCAL_ADAPTERS=0 \
CTX_BUNDLE_BUILD_CODEX_CRP=0 \
CTX_BUNDLE_BUILD_CLAUDE_CRP=0 \
CTX_BUNDLE_CLAUDE_CRP_WORKSPACE="$claude_ws" \
"$BUNDLE_SCRIPT"

python3 - "$bundle_dir/manifest.json" "$os" "$arch" <<'PY'
import json
import sys
from pathlib import Path

manifest_path = Path(sys.argv[1])
os_name = sys.argv[2]
arch = sys.argv[3]
manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
providers = manifest.get("providers") or []

entry = None
for provider in providers:
    if provider.get("id") == "claude-crp" and provider.get("os") == os_name and provider.get("arch") == arch:
        entry = provider
        break

if entry is None:
    print("missing claude-crp provider entry in manifest", file=sys.stderr)
    raise SystemExit(1)

if entry.get("protocol") != "crp":
    print(f"expected protocol=crp, found {entry.get('protocol')!r}", file=sys.stderr)
    raise SystemExit(1)

command = entry.get("command")
args = entry.get("args") or []
if not isinstance(command, str) or not command.startswith("runtimes/node/"):
    print(f"expected command to point at bundled node runtime, found {command!r}", file=sys.stderr)
    raise SystemExit(1)
if not args or not isinstance(args[0], str) or "providers/claude-crp/" not in args[0]:
    print(f"expected first arg to point at bundled claude-crp entrypoint, found {args!r}", file=sys.stderr)
    raise SystemExit(1)

bundle_root = manifest_path.parent
command_path = bundle_root / command
entrypoint_path = bundle_root / args[0]
runtime_path = entrypoint_path.parent.parent / "dist" / "runtime.js"

if not command_path.is_file():
    print(f"missing bundled command binary: {command_path}", file=sys.stderr)
    raise SystemExit(1)
if not entrypoint_path.is_file():
    print(f"missing bundled claude entrypoint: {entrypoint_path}", file=sys.stderr)
    raise SystemExit(1)
if not runtime_path.is_file():
    print(f"missing bundled claude runtime: {runtime_path}", file=sys.stderr)
    raise SystemExit(1)
PY

echo "ok: ensure_bundled_harnesses bundles claude-crp from local workspace"
