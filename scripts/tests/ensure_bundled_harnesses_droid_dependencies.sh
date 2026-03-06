#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BUNDLE_SCRIPT="$ROOT/scripts/ensure_bundled_harnesses.sh"

if [[ ! -x "$BUNDLE_SCRIPT" ]]; then
  echo "error: missing bundle script: $BUNDLE_SCRIPT" >&2
  exit 2
fi

for cmd in python3; do
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "error: $cmd is required for this regression test" >&2
    exit 2
  fi
done

tmp_root="$(mktemp -d /tmp/ctx-bundle-droid-managed.XXXXXX)"
trap 'rm -rf "$tmp_root"' EXIT

bundle_dir="$tmp_root/bundle"
payload_dir="$tmp_root/payload"
mkdir -p "$payload_dir/provider"

printf 'droid-acp test\n' > "$payload_dir/provider/droid-acp"
chmod +x "$payload_dir/provider/droid-acp"
printf 'droid cli test\n' > "$payload_dir/droid"
chmod +x "$payload_dir/droid"
printf 'rg test\n' > "$payload_dir/rg"
chmod +x "$payload_dir/rg"

provider_archive="$tmp_root/droid-provider.tar.gz"
(cd "$payload_dir/provider" && tar -czf "$provider_archive" .)

sha256() {
  python3 - "$1" <<'PY'
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
}

provider_sha="$(sha256 "$provider_archive")"
droid_sha="$(sha256 "$payload_dir/droid")"
rg_sha="$(sha256 "$payload_dir/rg")"

os_raw="$(uname -s 2>/dev/null || true)"
case "$os_raw" in
  Darwin) os="macos"; matrix_os="darwin";;
  Linux) os="linux"; matrix_os="linux";;
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

matrix_path="$tmp_root/provider_matrix.json"
cat > "$matrix_path" <<JSON
{
  "version": 2,
  "providers": [
    {
      "id": "droid",
      "display_name": "Droid",
      "command": { "command": "droid-acp", "args": [] },
      "managed_install": {
        "kind": "archive",
        "version": "0.0.0-test",
        "args": [],
        "targets": {
          "${matrix_os}-${arch}": {
            "url": "file://${provider_archive}",
            "archive": "tar_gz",
            "bin_path": "droid-acp",
            "sha256": "${provider_sha}",
            "size_bytes": 1234
          }
        }
      },
      "dependencies": [
        {
          "id": "droid-cli",
          "install": {
            "kind": "archive",
            "version": "0.0.0-test",
            "targets": {
              "${matrix_os}-${arch}": {
                "url": "file://${payload_dir}/droid",
                "archive": "none",
                "bin_path": "droid",
                "sha256": "${droid_sha}",
                "size_bytes": 123
              }
            }
          }
        },
        {
          "id": "droid-rg",
          "install": {
            "kind": "archive",
            "version": "0.0.0-test",
            "targets": {
              "${matrix_os}-${arch}": {
                "url": "file://${payload_dir}/rg",
                "archive": "none",
                "bin_path": "rg",
                "sha256": "${rg_sha}",
                "size_bytes": 123
              }
            }
          }
        }
      ],
      "releases": [{ "version": "0.0.0-test", "status": "supported", "context_min": "0.1.0" }]
    }
  ]
}
JSON

CTX_BUNDLE_DIR="$bundle_dir" \
CTX_BUNDLE_MATRIX_JSON="$matrix_path" \
CTX_BUNDLE_ONLY_PROVIDERS="droid" \
CTX_BUNDLE_SKIP_RUNTIMES=1 \
CTX_BUNDLE_SKIP_IMAGES=1 \
CTX_BUNDLE_INCLUDE_BRIDGE=0 \
CTX_BUNDLE_LOCAL_ADAPTERS=off \
CTX_BUNDLE_BUILD_LOCAL_ADAPTERS=0 \
"$BUNDLE_SCRIPT" >/dev/null

python3 - "$bundle_dir/manifest.json" "$os" "$arch" <<'PY'
import json
import sys
from pathlib import Path

manifest_path = Path(sys.argv[1])
os_name = sys.argv[2]
arch = sys.argv[3]
manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
providers = manifest.get("providers") or []

if len(providers) != 1:
    print(f"expected exactly one provider entry, got {len(providers)}", file=sys.stderr)
    raise SystemExit(1)

entry = providers[0]
if entry.get("id") != "droid":
    print(f"expected droid provider entry, got {entry.get('id')!r}", file=sys.stderr)
    raise SystemExit(1)

command_rel = str(entry.get("command") or "")
bundle_root = manifest_path.parent
command_path = bundle_root / command_rel
provider_root = bundle_root / "providers" / "droid" / os_name / arch
droid_path = provider_root / "droid"
rg_path = provider_root / "rg"

if not command_path.is_file():
    print(f"bundled droid-acp missing: {command_path}", file=sys.stderr)
    raise SystemExit(1)
if command_path.read_text(encoding="utf-8").strip() != "droid-acp test":
    print("unexpected droid-acp payload", file=sys.stderr)
    raise SystemExit(1)
if not droid_path.is_file():
    print(f"bundled droid dependency missing: {droid_path}", file=sys.stderr)
    raise SystemExit(1)
if not rg_path.is_file():
    print(f"bundled ripgrep dependency missing: {rg_path}", file=sys.stderr)
    raise SystemExit(1)
if droid_path.read_text(encoding="utf-8").strip() != "droid cli test":
    print("unexpected droid CLI payload", file=sys.stderr)
    raise SystemExit(1)
if rg_path.read_text(encoding="utf-8").strip() != "rg test":
    print("unexpected rg payload", file=sys.stderr)
    raise SystemExit(1)
PY

echo "ok: ensure_bundled_harnesses co-locates droid managed archive dependencies"
