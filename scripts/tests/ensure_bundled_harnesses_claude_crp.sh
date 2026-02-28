#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BUNDLE_SCRIPT="$ROOT/scripts/ensure_bundled_harnesses.sh"

if [[ ! -x "$BUNDLE_SCRIPT" ]]; then
  echo "error: missing bundle script: $BUNDLE_SCRIPT" >&2
  exit 2
fi

for cmd in node python3 gzip tar; do
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "error: $cmd is required for this regression test" >&2
    exit 2
  fi
done

tmp_root="$(mktemp -d /tmp/ctx-bundle-claude-crp.XXXXXX)"
trap 'rm -rf "$tmp_root"' EXIT

bundle_dir="$tmp_root/bundle"
provider_root="$tmp_root/provider-root"
archive_dir="$tmp_root/archive"
mkdir -p "$provider_root/bin" "$provider_root/dist" "$archive_dir"
mkdir -p "$provider_root/node_modules/@anthropic-ai/claude-agent-sdk/vendor/ripgrep/arm64-darwin"
mkdir -p "$provider_root/node_modules/@anthropic-ai/claude-code/vendor/ripgrep/arm64-darwin"

cat > "$provider_root/package.json" <<'JSON'
{
  "name": "claude-crp",
  "version": "0.0.0-test",
  "private": true,
  "type": "module"
}
JSON

cat > "$provider_root/bin/claude-crp" <<'JS'
#!/usr/bin/env node
import "../dist/runtime.js";
console.log("claude-crp test");
JS
chmod +x "$provider_root/bin/claude-crp"

cat > "$provider_root/dist/runtime.js" <<'JS'
export const runtime = "ok";
JS

printf 'rg\n' > "$provider_root/node_modules/@anthropic-ai/claude-agent-sdk/vendor/ripgrep/arm64-darwin/rg"
printf 'rg\n' > "$provider_root/node_modules/@anthropic-ai/claude-code/vendor/ripgrep/arm64-darwin/rg"

archive_path="$archive_dir/claude-crp-test.tar.gz"
(cd "$provider_root" && tar -czf "$archive_path" .)

archive_sha="$(python3 - "$archive_path" <<'PY'
import hashlib
import sys
from pathlib import Path

p = Path(sys.argv[1])
h = hashlib.sha256()
with p.open("rb") as fh:
    while True:
        chunk = fh.read(1024 * 1024)
        if not chunk:
            break
        h.update(chunk)
print(h.hexdigest())
PY
)"

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
  Darwin) os="macos"; matrix_os="darwin"; node_target="darwin-arm64";;
  Linux) os="linux"; matrix_os="linux"; node_target="linux-arm64";;
  *)
    echo "error: unsupported host os for this test: $os_raw" >&2
    exit 2
    ;;
esac

arch_raw="$(uname -m 2>/dev/null || true)"
case "$arch_raw" in
  x86_64|amd64) arch="x86_64";;
  aarch64|arm64) arch="aarch64";;
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

ln -s "$node_bin_src" "$node_root/bin/node"
cat > "$node_root/lib/node_modules/npm/bin/npm-cli.js" <<'JS'
#!/usr/bin/env node
console.log("npm cli placeholder");
JS
chmod +x "$node_root/lib/node_modules/npm/bin/npm-cli.js"

matrix_path="$tmp_root/provider_matrix.json"
cat > "$matrix_path" <<JSON
{
  "version": 2,
  "providers": [
    {
      "id": "claude-crp",
      "display_name": "Claude",
      "tier": "tier2",
      "command": { "command": "claude-crp", "args": [] },
      "managed_install": {
        "kind": "archive",
        "version": "0.0.0-test",
        "args": [],
        "targets": {
          "${matrix_os}-${arch}": {
            "url": "file://${archive_path}",
            "archive": "tar_gz",
            "bin_path": "bin/claude-crp",
            "sha256": "${archive_sha}"
          }
        }
      },
      "releases": [{ "version": "0.0.0-test", "status": "supported", "context_min": "0.1.0" }]
    }
  ]
}
JSON

CTX_BUNDLE_DIR="$bundle_dir" \
CTX_BUNDLE_MATRIX_JSON="$matrix_path" \
CTX_BUNDLE_ONLY_PROVIDERS="claude-crp" \
CTX_BUNDLE_SKIP_IMAGES=1 \
CTX_BUNDLE_INCLUDE_BRIDGE=0 \
CTX_BUNDLE_LOCAL_ADAPTERS=off \
CTX_BUNDLE_BUILD_LOCAL_ADAPTERS=0 \
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

echo "ok: ensure_bundled_harnesses bundles claude-crp from managed archive target only"
