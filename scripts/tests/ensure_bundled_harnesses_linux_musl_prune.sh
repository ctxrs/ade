#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BUNDLE_SCRIPT="$ROOT/scripts/ensure_bundled_harnesses.sh"

if [[ ! -x "$BUNDLE_SCRIPT" ]]; then
  echo "error: missing bundle script: $BUNDLE_SCRIPT" >&2
  exit 2
fi

for cmd in node python3 tar; do
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "error: $cmd is required for this regression test" >&2
    exit 2
  fi
done

tmp_root="$(mktemp -d /tmp/ctx-bundle-linux-musl-prune.XXXXXX)"
trap 'rm -rf "$tmp_root"' EXIT

bundle_dir="$tmp_root/bundle"
provider_root="$tmp_root/provider-root"
archive_dir="$tmp_root/archive"
mkdir -p "$provider_root/bin" "$provider_root/dist" "$archive_dir"
mkdir -p "$provider_root/node_modules/@napi-rs/keyring-linux-x64-gnu/lib"
mkdir -p "$provider_root/node_modules/@napi-rs/keyring-linux-x64-musl/lib"

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

printf 'gnu-node\n' > "$provider_root/node_modules/@napi-rs/keyring-linux-x64-gnu/lib/keyring.linux-x64-gnu.node"
printf 'musl-node\n' > "$provider_root/node_modules/@napi-rs/keyring-linux-x64-musl/lib/keyring.linux-x64-musl.node"

archive_path="$archive_dir/claude-crp-test-linux.tar.gz"
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

node_version="$(python3 - "$ROOT/core/crates/ctx-managed-installs/src/lib.rs" <<'PY'
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
          "linux-x86_64": {
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
CTX_BUNDLE_OS=linux \
CTX_BUNDLE_ARCH=x86_64 \
CTX_BUNDLE_ONLY_PROVIDERS="claude-crp" \
CTX_BUNDLE_SKIP_IMAGES=1 \
CTX_BUNDLE_INCLUDE_BRIDGE=0 \
CTX_BUNDLE_LOCAL_ADAPTERS=off \
CTX_BUNDLE_BUILD_LOCAL_ADAPTERS=0 \
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

echo "ok: ensure_bundled_harnesses prunes linux musl keyring payloads for managed claude-crp archive"
