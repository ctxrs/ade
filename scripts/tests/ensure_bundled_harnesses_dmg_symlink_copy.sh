#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BUNDLE_SCRIPT="$ROOT/scripts/ensure_bundled_harnesses.sh"

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "skip: dmg symlink regression test runs on macOS only"
  exit 0
fi

if [[ ! -x "$BUNDLE_SCRIPT" ]]; then
  echo "error: missing bundle script: $BUNDLE_SCRIPT" >&2
  exit 2
fi

if ! command -v hdiutil >/dev/null 2>&1; then
  echo "error: hdiutil is required for dmg symlink regression test" >&2
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

tmp_root="$(mktemp -d /tmp/ctx-bundle-dmg-symlink.XXXXXX)"
trap 'rm -rf "$tmp_root"' EXIT

host_apps="$tmp_root/host-apps"
mkdir -p "$host_apps/Fake.app/Contents/_CodeSignature"
printf 'host-content\n' > "$host_apps/Fake.app/Contents/_CodeSignature/CodeDirectory"

dmg_src="$tmp_root/dmg-src"
mkdir -p "$dmg_src/kiro/bin"
printf '#!/usr/bin/env bash\necho kiro\n' > "$dmg_src/kiro/bin/kiro-cli"
chmod +x "$dmg_src/kiro/bin/kiro-cli"
ln -s "$host_apps" "$dmg_src/Applications"

dmg_path="$tmp_root/kiro.dmg"
hdiutil create \
  -quiet \
  -volname "ctx-dmg-test" \
  -srcfolder "$dmg_src" \
  -ov \
  -format UDZO \
  "$dmg_path"

matrix_path="$tmp_root/provider_matrix.json"
cat > "$matrix_path" <<EOF
{
  "version": 2,
  "providers": [
    {
      "id": "kiro",
      "kind": "archive",
      "managed_install": {
        "kind": "archive",
        "version": "test-dmg-symlink",
        "source": "ci",
        "targets": {
          "darwin-aarch64": {
            "url": "file://$dmg_path",
            "archive": "dmg",
            "bin_path": "kiro/bin/kiro-cli"
          }
        }
      }
    }
  ]
}
EOF

bundle_dir="$tmp_root/bundle"
CTX_BUNDLE_DIR="$bundle_dir" \
CTX_BUNDLE_ONLY_PROVIDERS="kiro" \
CTX_BUNDLE_MATRIX_JSON="$matrix_path" \
CTX_BUNDLE_OS="macos" \
CTX_BUNDLE_ARCH="aarch64" \
CTX_BUNDLE_SKIP_RUNTIMES=1 \
CTX_BUNDLE_SKIP_IMAGES=1 \
CTX_BUNDLE_INCLUDE_BRIDGE=0 \
CTX_BUNDLE_LOCAL_ADAPTERS=off \
CTX_BUNDLE_BUILD_LOCAL_ADAPTERS=0 \
"$BUNDLE_SCRIPT" >/dev/null

if ! "$python_cmd" - "$bundle_dir/manifest.json" <<'PY'
import json
import os
import sys
from pathlib import Path

manifest_path = Path(sys.argv[1])
data = json.loads(manifest_path.read_text(encoding="utf-8"))
providers = data.get("providers") or []
if len(providers) != 1:
    print(f"expected one provider entry, got {len(providers)}", file=sys.stderr)
    raise SystemExit(1)

entry = providers[0]
if entry.get("id") != "kiro":
    print(f"expected kiro provider entry, got {entry.get('id')!r}", file=sys.stderr)
    raise SystemExit(1)

bundle_root = manifest_path.parent
provider_root = bundle_root / "providers" / "kiro" / "macos" / "aarch64"
if not provider_root.exists():
    print(f"missing provider root: {provider_root}", file=sys.stderr)
    raise SystemExit(1)

command = bundle_root / str(entry.get("command", ""))
if not command.is_file():
    print(f"missing command path: {command}", file=sys.stderr)
    raise SystemExit(1)

applications = provider_root / "Applications"
if applications.exists():
    if applications.is_symlink():
        pass
    elif applications.is_dir():
        print(f"Applications was dereferenced into directory: {applications}", file=sys.stderr)
        raise SystemExit(1)
    else:
        print(f"Applications exists but is not symlink: {applications}", file=sys.stderr)
        raise SystemExit(1)

if (
    applications.exists()
    and not applications.is_symlink()
    and (provider_root / "Applications" / "Fake.app" / "Contents" / "_CodeSignature" / "CodeDirectory").exists()
):
    print("host-app content was copied through Applications symlink", file=sys.stderr)
    raise SystemExit(1)
PY
then
  echo "error: dmg symlink copy regression validation failed" >&2
  exit 1
fi

echo "ok: ensure_bundled_harnesses preserves dmg symlinks without dereferencing host application trees"
