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
adapters_dir="$tmp_root/adapters"
bin_dir="$tmp_root/bin"
workspace_dir="$tmp_root/local-codex-workspace"
mkdir -p "$archive_dir" "$workspace_dir" "$adapters_dir" "$bin_dir"

cat > "$bin_dir/cargo" <<'SH'
#!/usr/bin/env bash
echo "unexpected cargo invocation for codex-only bundle" >&2
exit 97
SH
chmod +x "$bin_dir/cargo"

provider_root="$archive_dir/payload"
mkdir -p "$provider_root"
printf 'managed codex\n' > "$provider_root/codex-crp"
chmod +x "$provider_root/codex-crp"

dependency_root="$archive_dir/dependency"
mkdir -p "$dependency_root"
printf 'managed codex cli\n' > "$dependency_root/codex"
chmod +x "$dependency_root/codex"

archive_path="$archive_dir/codex-crp-test.tar.gz"
(cd "$provider_root" && tar -czf "$archive_path" .)

dependency_archive_path="$archive_dir/codex-cli-test.tar.gz"
(cd "$dependency_root" && tar -czf "$dependency_archive_path" .)

archive_sha="$(python3 -c 'import hashlib, pathlib, sys; p=pathlib.Path(sys.argv[1]); h=hashlib.sha256(); h.update(p.read_bytes()); print(h.hexdigest())' "$archive_path")"
dependency_archive_sha="$(python3 -c 'import hashlib, pathlib, sys; p=pathlib.Path(sys.argv[1]); h=hashlib.sha256(); h.update(p.read_bytes()); print(h.hexdigest())' "$dependency_archive_path")"

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
      "id": "codex",
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
      "provider_dependencies": [
        {
          "id": "codex-cli",
          "role": "prerequisite",
          "target": "same_as_provider"
        }
      ],
      "releases": [{ "version": "0.0.0-test", "status": "supported", "context_min": "0.1.0" }]
    },
    {
      "id": "codex-cli",
      "display_name": "Codex CLI",
      "tier": "tier2",
      "command": { "command": "codex", "args": [] },
      "managed_install": {
        "kind": "archive",
        "version": "0.0.0-test",
        "args": [],
        "targets": {
          "${matrix_os}-${arch}": {
            "url": "file://${dependency_archive_path}",
            "archive": "tar_gz",
            "bin_path": "codex",
            "sha256": "${dependency_archive_sha}"
          }
        }
      },
      "releases": [{ "version": "0.0.0-test", "status": "supported", "context_min": "0.1.0" }]
    }
  ]
}
JSON

PATH="$bin_dir:$PATH" \
CTX_BUNDLE_DIR="$bundle_dir" \
CTX_BUNDLE_MATRIX_JSON="$matrix_path" \
CTX_BUNDLE_ONLY_PROVIDERS="codex" \
CTX_BUNDLE_SKIP_RUNTIMES=1 \
CTX_BUNDLE_SKIP_IMAGES=1 \
CTX_BUNDLE_INCLUDE_BRIDGE=0 \
CTX_BUNDLE_LOCAL_ADAPTERS=on \
CTX_BUNDLE_BUILD_LOCAL_ADAPTERS=1 \
CTX_BUNDLE_ADAPTERS_DIR="$adapters_dir" \
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
entries = {
    str(entry.get("id") or ""): entry
    for entry in providers
}
if set(entries) != {"codex", "codex-cli"}:
    print(f"expected codex and codex-cli provider entries, got {sorted(entries)}", file=sys.stderr)
    raise SystemExit(1)

entry = entries["codex"]
dep_entry = entries["codex-cli"]
for current in (entry, dep_entry):
    if current.get("version") != "0.0.0-test":
        print(f"expected version 0.0.0-test, got {current.get('id')!r}={current.get('version')!r}", file=sys.stderr)
        raise SystemExit(1)
if current.get("os") != os_name or current.get("arch") != arch:
        print(f"unexpected target tuple for {current.get('id')!r}: {current.get('os')!r}/{current.get('arch')!r}", file=sys.stderr)
        raise SystemExit(1)

command_rel = str(entry.get("command") or "")
dep_command_rel = str(dep_entry.get("command") or "")
if not command_rel.startswith(f"providers/codex/{os_name}/{arch}/"):
    print(f"expected managed codex bundle path, got {command_rel!r}", file=sys.stderr)
    raise SystemExit(1)
if not dep_command_rel.startswith(f"providers/codex-cli/{os_name}/{arch}/"):
    print(f"expected managed codex-cli bundle path, got {dep_command_rel!r}", file=sys.stderr)
    raise SystemExit(1)

command_path = manifest_path.parent / command_rel
dep_command_path = manifest_path.parent / dep_command_rel
if not command_path.is_file():
    print(f"bundled codex binary missing: {command_path}", file=sys.stderr)
    raise SystemExit(1)
if command_path.read_text(encoding="utf-8").strip() != "managed codex":
    print(f"unexpected codex payload contents: {command_path.read_text(encoding='utf-8')!r}", file=sys.stderr)
    raise SystemExit(1)
if not dep_command_path.is_file():
    print(f"bundled codex-cli binary missing: {dep_command_path}", file=sys.stderr)
    raise SystemExit(1)
if dep_command_path.read_text(encoding="utf-8").strip() != "managed codex cli":
    print(f"unexpected codex-cli payload contents: {dep_command_path.read_text(encoding='utf-8')!r}", file=sys.stderr)
    raise SystemExit(1)
PY

echo "ok: ensure_bundled_harnesses expands codex managed prerequisites by default"
