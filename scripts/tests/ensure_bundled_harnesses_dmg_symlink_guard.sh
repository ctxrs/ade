#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BUNDLE_SCRIPT="$ROOT/scripts/ensure_bundled_harnesses.sh"

if [[ ! -x "$BUNDLE_SCRIPT" ]]; then
  echo "error: missing bundle script: $BUNDLE_SCRIPT" >&2
  exit 2
fi

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "skip: DMG symlink guard regression test requires macOS"
  exit 0
fi

if ! command -v hdiutil >/dev/null 2>&1; then
  echo "error: hdiutil is required for this regression test" >&2
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

host_arch_raw="$(uname -m 2>/dev/null || true)"
case "$host_arch_raw" in
  x86_64|amd64)
    matrix_target="darwin-x86_64"
    arch_label="x86_64"
    ;;
  aarch64|arm64)
    matrix_target="darwin-aarch64"
    arch_label="aarch64"
    ;;
  *)
    echo "error: unsupported host arch: $host_arch_raw" >&2
    exit 2
    ;;
esac

tmp_root="$(mktemp -d /tmp/ctx-bundle-dmg-symlink-guard.XXXXXX)"
trap 'rm -rf "$tmp_root"' EXIT

payload_dir="$tmp_root/payload"
mkdir -p "$payload_dir"
cat >"$payload_dir/kiro-cli" <<'SH'
#!/usr/bin/env bash
echo "kiro test cli"
SH
chmod +x "$payload_dir/kiro-cli"
ln -s /Applications "$payload_dir/Applications"

dmg_path="$tmp_root/test-kiro-cli.dmg"
hdiutil create -quiet -ov -volname "TestKiroCLI" -srcfolder "$payload_dir" "$dmg_path"

matrix_path="$tmp_root/provider_matrix.json"
"$python_cmd" - "$matrix_path" "$dmg_path" "$matrix_target" <<'PY'
import json
import sys
from pathlib import Path

matrix_path = Path(sys.argv[1])
dmg_path = Path(sys.argv[2]).resolve()
target = sys.argv[3]

matrix = {
    "providers": [
        {
            "id": "kiro",
            "display_name": "Kiro",
            "managed_install": {
                "kind": "archive",
                "version": "1.26.2",
                "args": ["acp"],
                "targets": {
                    target: {
                        "url": dmg_path.as_uri(),
                        "archive": "dmg",
                        "bin_path": "kiro-cli",
                    }
                },
            },
            "releases": [
                {
                    "version": "1.26.2",
                    "status": "supported",
                    "context_min": "0.1.0",
                }
            ],
        }
    ]
}

matrix_path.write_text(json.dumps(matrix, indent=2), encoding="utf-8")
PY

bundle_dir="$tmp_root/bundle"
CTX_BUNDLE_MATRIX_JSON="$matrix_path" \
CTX_BUNDLE_DIR="$bundle_dir" \
CTX_BUNDLE_ONLY_PROVIDERS="kiro" \
CTX_BUNDLE_SKIP_RUNTIMES=1 \
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
if len(providers) != 1:
    print(f"expected exactly one provider entry, got {len(providers)}", file=sys.stderr)
    raise SystemExit(1)
entry = providers[0]
if entry.get("id") != "kiro":
    print(f"expected kiro provider entry, got {entry.get('id')!r}", file=sys.stderr)
    raise SystemExit(1)
command_rel = str(entry.get("command", ""))
cmd_path = manifest_path.parent / command_rel
if not cmd_path.is_file():
    print(f"expected bundled command file missing: {cmd_path}", file=sys.stderr)
    raise SystemExit(1)
PY
then
  echo "error: manifest validation failed for DMG symlink guard regression test" >&2
  exit 1
fi

if [[ -e "$bundle_dir/providers/kiro/macos/$arch_label/Applications" ]]; then
  echo "error: external Applications symlink should not be copied into provider bundle" >&2
  exit 1
fi

echo "ok: ensure_bundled_harnesses skips external symlinks when copying DMG payloads"
