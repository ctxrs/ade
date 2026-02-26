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

tmp_root="$(mktemp -d /tmp/ctx-bundle-linux-musl-prune.XXXXXX)"
trap 'rm -rf "$tmp_root"' EXIT

bundle_dir="$tmp_root/bundle"
claude_ws="$tmp_root/claude-crp"
gnu_pkg="$tmp_root/keyring-linux-x64-gnu"
musl_pkg="$tmp_root/keyring-linux-x64-musl"
mkdir -p "$claude_ws/bin" "$claude_ws/dist" "$gnu_pkg" "$musl_pkg"

cat > "$gnu_pkg/package.json" <<'JSON'
{
  "name": "@napi-rs/keyring-linux-x64-gnu",
  "version": "0.0.0-test",
  "private": true
}
JSON
printf 'gnu\n' > "$gnu_pkg/index.js"
mkdir -p "$gnu_pkg/lib"
printf 'gnu-node\n' > "$gnu_pkg/lib/keyring.linux-x64-gnu.node"

cat > "$musl_pkg/package.json" <<'JSON'
{
  "name": "@napi-rs/keyring-linux-x64-musl",
  "version": "0.0.0-test",
  "private": true
}
JSON
printf 'musl\n' > "$musl_pkg/index.js"
mkdir -p "$musl_pkg/lib"
printf 'musl-node\n' > "$musl_pkg/lib/keyring.linux-x64-musl.node"

cat > "$claude_ws/package.json" <<'JSON'
{
  "name": "claude-crp",
  "version": "0.0.0-test",
  "private": true,
  "type": "module",
  "dependencies": {
    "@napi-rs/keyring-linux-x64-gnu": "file:__GNU_PKG__",
    "@napi-rs/keyring-linux-x64-musl": "file:__MUSL_PKG__"
  }
}
JSON
sed -i.bak "s#__GNU_PKG__#${gnu_pkg}#g; s#__MUSL_PKG__#${musl_pkg}#g" "$claude_ws/package.json"
rm -f "$claude_ws/package.json.bak"

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

node_root="$bundle_dir/runtimes/node/linux/x86_64/node-v${node_version}-linux-x64"
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
CTX_BUNDLE_OS=linux \
CTX_BUNDLE_ARCH=x86_64 \
CTX_BUNDLE_ONLY_PROVIDERS="claude-crp" \
CTX_BUNDLE_SKIP_IMAGES=1 \
CTX_BUNDLE_INCLUDE_BRIDGE=0 \
CTX_BUNDLE_LOCAL_ADAPTERS=off \
CTX_BUNDLE_BUILD_LOCAL_ADAPTERS=0 \
CTX_BUNDLE_BUILD_CLAUDE_CRP=0 \
CTX_BUNDLE_CLAUDE_CRP_WORKSPACE="$claude_ws" \
"$BUNDLE_SCRIPT"

python3 - "$bundle_dir" <<'PY'
import json
import sys
from pathlib import Path

bundle = Path(sys.argv[1])
manifest = json.loads((bundle / "manifest.json").read_text(encoding="utf-8"))
providers = manifest.get("providers") or []
entry = next((p for p in providers if p.get("id") == "claude-crp" and p.get("os") == "linux" and p.get("arch") == "x86_64"), None)
if entry is None:
    print("missing linux/x86_64 claude-crp provider entry", file=sys.stderr)
    raise SystemExit(1)

provider_root = bundle / "providers" / "claude-crp" / "linux" / "x86_64" / "node_modules" / "@napi-rs"
gnu_dir = provider_root / "keyring-linux-x64-gnu"
musl_dir = provider_root / "keyring-linux-x64-musl"

if not gnu_dir.is_dir():
    print(f"expected gnu keyring package at {gnu_dir}", file=sys.stderr)
    raise SystemExit(1)
if musl_dir.exists():
    print(f"musl keyring package should be pruned but still exists at {musl_dir}", file=sys.stderr)
    raise SystemExit(1)
PY

echo "ok: ensure_bundled_harnesses prunes linux musl keyring payloads"
