#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
MERGE_SCRIPT="$ROOT/core/scripts/desktop_merge_bundles.cjs"

if [[ ! -f "$MERGE_SCRIPT" ]]; then
  echo "error: missing merge script: $MERGE_SCRIPT" >&2
  exit 2
fi

tmp_root="$(mktemp -d /tmp/ctx-desktop-bundle-merge.XXXXXX)"
trap 'rm -rf "$tmp_root"' EXIT

bundle_x64="$tmp_root/x64"
bundle_arm64="$tmp_root/arm64"
bundle_out="$tmp_root/out"
bundle_out_single="$tmp_root/out-single"
mkdir -p "$bundle_x64/providers/codex-crp/linux/x86_64"
mkdir -p "$bundle_arm64/providers/codex-crp/linux/aarch64"
mkdir -p "$bundle_x64/images" "$bundle_arm64/images"
mkdir -p "$bundle_x64/daemons" "$bundle_arm64/daemons"

printf 'codex-x64' > "$bundle_x64/providers/codex-crp/linux/x86_64/codex-crp"
printf 'codex-arm64' > "$bundle_arm64/providers/codex-crp/linux/aarch64/codex-crp"
printf 'harness-x64' > "$bundle_x64/images/ctx-harness-linux-x86_64.tar"
printf 'harness-arm64' > "$bundle_arm64/images/ctx-harness-linux-aarch64.tar"
printf 'daemon-x64' > "$bundle_x64/daemons/ctx-daemon-linux-x86_64"
printf 'daemon-arm64' > "$bundle_arm64/daemons/ctx-daemon-linux-aarch64"

cat > "$bundle_x64/manifest.json" <<'JSON'
{
  "version": 1,
  "providers": [
    {
      "id": "codex-crp",
      "protocol": "crp",
      "version": "0.0.0-ctx.3",
      "os": "linux",
      "arch": "x86_64",
      "sha256": "x64-sha",
      "command": "providers/codex/linux/x86_64/codex-crp",
      "args": []
    }
  ],
  "daemons": [
    {
      "id": "ctx-daemon",
      "os": "linux",
      "arch": "x86_64",
      "sha256": "x64-daemon-sha",
      "bin": "daemons/ctx-daemon-linux-x86_64"
    }
  ],
  "runtimes": [],
  "images": [
    {
      "id": "ctx-harness",
      "version": "ctx-harness:latest",
      "os": "linux",
      "arch": "x86_64",
      "sha256": "x64-image-sha",
      "tar": "images/ctx-harness-linux-x86_64.tar",
      "image": "ctx-harness:latest"
    }
  ]
}
JSON

cat > "$bundle_arm64/manifest.json" <<'JSON'
{
  "version": 1,
  "providers": [
    {
      "id": "codex-crp",
      "protocol": "crp",
      "version": "0.0.0-ctx.3",
      "os": "linux",
      "arch": "aarch64",
      "sha256": "arm64-sha",
      "command": "providers/codex/linux/aarch64/codex-crp",
      "args": []
    }
  ],
  "daemons": [
    {
      "id": "ctx-daemon",
      "os": "linux",
      "arch": "aarch64",
      "sha256": "arm64-daemon-sha",
      "bin": "daemons/ctx-daemon-linux-aarch64"
    }
  ],
  "runtimes": [],
  "images": [
    {
      "id": "ctx-harness",
      "version": "ctx-harness:latest",
      "os": "linux",
      "arch": "aarch64",
      "sha256": "arm64-image-sha",
      "tar": "images/ctx-harness-linux-aarch64.tar",
      "image": "ctx-harness:latest"
    }
  ]
}
JSON

node "$MERGE_SCRIPT" \
  --input "$bundle_x64" \
  --input "$bundle_arm64" \
  --output "$bundle_out" \
  --require-provider codex-crp:linux:x86_64 \
  --require-provider codex-crp:linux:aarch64 \
  --require-image ctx-harness:linux:x86_64 \
  --require-image ctx-harness:linux:aarch64 \
  --require-daemon ctx-daemon:linux:x86_64 \
  --require-daemon ctx-daemon:linux:aarch64

python3 - "$bundle_out/manifest.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
data = json.loads(path.read_text(encoding="utf-8"))
providers = data.get("providers") or []
images = data.get("images") or []
daemons = data.get("daemons") or []
if len(providers) != 2:
    raise SystemExit(f"expected 2 providers, got {len(providers)}")
if len(images) != 2:
    raise SystemExit(f"expected 2 images, got {len(images)}")
if len(daemons) != 2:
    raise SystemExit(f"expected 2 daemons, got {len(daemons)}")
PY

node "$MERGE_SCRIPT" \
  --input "$bundle_x64" \
  --output "$bundle_out_single" \
  --require-provider codex-crp:linux:x86_64 \
  --require-image ctx-harness:linux:x86_64 \
  --require-daemon ctx-daemon:linux:x86_64

python3 - "$bundle_out_single/manifest.json" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
data = json.loads(path.read_text(encoding="utf-8"))
providers = data.get("providers") or []
images = data.get("images") or []
daemons = data.get("daemons") or []
if len(providers) != 1:
    raise SystemExit(f"expected 1 provider, got {len(providers)}")
if len(images) != 1:
    raise SystemExit(f"expected 1 image, got {len(images)}")
if len(daemons) != 1:
    raise SystemExit(f"expected 1 daemon, got {len(daemons)}")
PY

conflict_dir="$tmp_root/conflict"
mkdir -p "$conflict_dir/providers/codex-crp/linux/x86_64"
printf 'codex-x64-conflict' > "$conflict_dir/providers/codex-crp/linux/x86_64/codex-crp"
cat > "$conflict_dir/manifest.json" <<'JSON'
{
  "version": 1,
  "providers": [
    {
      "id": "codex-crp",
      "protocol": "crp",
      "version": "0.0.0-ctx.3",
      "os": "linux",
      "arch": "x86_64",
      "sha256": "different-sha",
      "command": "providers/codex-crp/linux/x86_64/codex-crp",
      "args": []
    }
  ],
  "daemons": [],
  "runtimes": [],
  "images": []
}
JSON

if node "$MERGE_SCRIPT" --input "$bundle_x64" --input "$conflict_dir" --output "$tmp_root/conflict-out" >/dev/null 2>&1; then
  echo "error: expected merge conflict to fail, but it succeeded" >&2
  exit 1
fi

echo "ok: desktop bundle merge script"
