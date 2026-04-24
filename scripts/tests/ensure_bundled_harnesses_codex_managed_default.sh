#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BUNDLE_SCRIPT="$ROOT/scripts/ensure_bundled_harnesses.sh"

if [[ ! -x "$BUNDLE_SCRIPT" ]]; then
  echo "error: missing bundle script: $BUNDLE_SCRIPT" >&2
  exit 2
fi

for cmd in python3 tar gzip; do
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "error: $cmd is required for this regression test" >&2
    exit 2
  fi
done

tmp_root="$(mktemp -d /tmp/ctx-bundle-codex-managed.XXXXXX)"
trap 'rm -rf "$tmp_root"' EXIT

bundle_dir="$tmp_root/bundle"
archive_dir="$tmp_root/archive"
workspace_dir="$tmp_root/local-codex-workspace"
mkdir -p "$archive_dir" "$workspace_dir"

provider_root="$archive_dir/payload"
mkdir -p "$provider_root"
printf 'managed codex\n' > "$provider_root/codex-crp"
chmod +x "$provider_root/codex-crp"

archive_path="$archive_dir/codex-crp-test.tar.gz"
(cd "$provider_root" && tar -czf "$archive_path" .)

archive_sha="$(python3 -c 'import hashlib, pathlib, sys; p=pathlib.Path(sys.argv[1]); h=hashlib.sha256(); h.update(p.read_bytes()); print(h.hexdigest())' "$archive_path")"

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
      "id": "codex-crp",
      "display_name": "Codex",
      "tier": "tier2",
      "command": { "command": "codex-crp", "args": [] },
      "managed_install": {
        "kind": "archive",
        "version": "0.0.0-test",
        "args": [],
        "targets": {
          "${matrix_os}-${arch}": {
            "url": "file://${archive_path}",
            "archive": "tar_gz",
            "bin_path": "codex-crp",
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
CTX_BUNDLE_ONLY_PROVIDERS="codex-crp" \
CTX_BUNDLE_SKIP_RUNTIMES=1 \
CTX_BUNDLE_SKIP_IMAGES=1 \
CTX_BUNDLE_INCLUDE_BRIDGE=0 \
CTX_BUNDLE_LOCAL_ADAPTERS=off \
CTX_BUNDLE_BUILD_LOCAL_ADAPTERS=0 \
CTX_BUNDLE_CODEX_CRP_WORKSPACE="$workspace_dir" \
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
if entry.get("id") != "codex-crp":
    print(f"expected codex provider entry, got {entry.get('id')!r}", file=sys.stderr)
    raise SystemExit(1)
if entry.get("version") != "0.0.0-test":
    print(f"expected codex version 0.0.0-test, got {entry.get('version')!r}", file=sys.stderr)
    raise SystemExit(1)
if entry.get("os") != os_name or entry.get("arch") != arch:
    print(f"unexpected target tuple: {entry.get('os')!r}/{entry.get('arch')!r}", file=sys.stderr)
    raise SystemExit(1)

command_rel = str(entry.get("command") or "")
if not command_rel.startswith(f"providers/codex/{os_name}/{arch}/"):
    print(f"expected managed codex bundle path, got {command_rel!r}", file=sys.stderr)
    raise SystemExit(1)

command_path = manifest_path.parent / command_rel
if not command_path.is_file():
    print(f"bundled codex binary missing: {command_path}", file=sys.stderr)
    raise SystemExit(1)
if command_path.read_text(encoding="utf-8").strip() != "managed codex":
    print(f"unexpected codex payload contents: {command_path.read_text(encoding='utf-8')!r}", file=sys.stderr)
    raise SystemExit(1)
PY

echo "ok: ensure_bundled_harnesses uses managed codex artifact by default"
