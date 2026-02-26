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
sdk_pkg="$tmp_root/claude-agent-sdk"
claude_code_pkg="$tmp_root/claude-code"
mkdir -p "$claude_ws/bin" "$claude_ws/dist"
mkdir -p "$sdk_pkg/vendor/ripgrep"
mkdir -p "$claude_code_pkg/vendor/ripgrep"

for target in x64-darwin arm64-darwin x64-linux arm64-linux x64-win32 arm64-win32; do
  mkdir -p "$sdk_pkg/vendor/ripgrep/$target"
  printf 'stub-%s\n' "$target" > "$sdk_pkg/vendor/ripgrep/$target/rg"
  printf 'stub-%s\n' "$target" > "$sdk_pkg/vendor/ripgrep/$target/ripgrep.node"
done

cat > "$sdk_pkg/package.json" <<'JSON'
{
  "name": "@anthropic-ai/claude-agent-sdk",
  "version": "0.2.7",
  "private": true,
  "type": "module"
}
JSON

for target in x64-darwin arm64-darwin x64-linux arm64-linux x64-win32 arm64-win32; do
  mkdir -p "$claude_code_pkg/vendor/ripgrep/$target"
  printf 'stub-%s\n' "$target" > "$claude_code_pkg/vendor/ripgrep/$target/rg"
  printf 'stub-%s\n' "$target" > "$claude_code_pkg/vendor/ripgrep/$target/ripgrep.node"
done

cat > "$claude_code_pkg/package.json" <<'JSON'
{
  "name": "@anthropic-ai/claude-code",
  "version": "2.1.47",
  "private": true,
  "type": "module"
}
JSON

cat > "$claude_ws/package.json" <<'JSON'
{
  "name": "claude-crp",
  "version": "0.0.0-test",
  "private": true,
  "type": "module",
  "dependencies": {
    "@anthropic-ai/claude-agent-sdk": "file:__SDK_PKG__",
    "@anthropic-ai/claude-code": "file:__CLAUDE_CODE_PKG__"
  }
}
JSON
sed -i.bak \
  "s#__SDK_PKG__#${sdk_pkg}#g; s#__CLAUDE_CODE_PKG__#${claude_code_pkg}#g" \
  "$claude_ws/package.json"
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
case "${os}/${arch}" in
  macos/x86_64) ripgrep_target="x64-darwin" ;;
  macos/aarch64) ripgrep_target="arm64-darwin" ;;
  linux/x86_64) ripgrep_target="x64-linux" ;;
  linux/aarch64) ripgrep_target="arm64-linux" ;;
  *)
    echo "error: unsupported ripgrep target for ${os}/${arch}" >&2
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
CTX_BUNDLE_BUILD_CLAUDE_CRP=0 \
CTX_BUNDLE_CLAUDE_CRP_WORKSPACE="$claude_ws" \
"$BUNDLE_SCRIPT"

python3 - "$bundle_dir/manifest.json" "$os" "$arch" "$ripgrep_target" <<'PY'
import json
import sys
from pathlib import Path

manifest_path = Path(sys.argv[1])
os_name = sys.argv[2]
arch = sys.argv[3]
expected_ripgrep_target = sys.argv[4]
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
ripgrep_vendor_sdk = entrypoint_path.parent.parent / "node_modules" / "@anthropic-ai" / "claude-agent-sdk" / "vendor" / "ripgrep"
ripgrep_vendor_claude_code = entrypoint_path.parent.parent / "node_modules" / "@anthropic-ai" / "claude-code" / "vendor" / "ripgrep"

if not command_path.is_file():
    print(f"missing bundled command binary: {command_path}", file=sys.stderr)
    raise SystemExit(1)
if not entrypoint_path.is_file():
    print(f"missing bundled claude entrypoint: {entrypoint_path}", file=sys.stderr)
    raise SystemExit(1)
if not runtime_path.is_file():
    print(f"missing bundled claude runtime: {runtime_path}", file=sys.stderr)
    raise SystemExit(1)
if not ripgrep_vendor_sdk.is_dir():
    print(f"missing bundled claude-agent-sdk ripgrep vendor dir: {ripgrep_vendor_sdk}", file=sys.stderr)
    raise SystemExit(1)
if not ripgrep_vendor_claude_code.is_dir():
    print(f"missing bundled claude-code ripgrep vendor dir: {ripgrep_vendor_claude_code}", file=sys.stderr)
    raise SystemExit(1)

actual_targets_sdk = sorted([p.name for p in ripgrep_vendor_sdk.iterdir() if p.is_dir()])
if actual_targets_sdk != [expected_ripgrep_target]:
    print(
        f"expected only claude-agent-sdk ripgrep target {expected_ripgrep_target!r}, found {actual_targets_sdk!r}",
        file=sys.stderr,
    )
    raise SystemExit(1)

actual_targets_claude_code = sorted([p.name for p in ripgrep_vendor_claude_code.iterdir() if p.is_dir()])
if actual_targets_claude_code != [expected_ripgrep_target]:
    print(
        f"expected only claude-code ripgrep target {expected_ripgrep_target!r}, found {actual_targets_claude_code!r}",
        file=sys.stderr,
    )
    raise SystemExit(1)
PY

echo "ok: ensure_bundled_harnesses bundles claude-crp from local workspace"
