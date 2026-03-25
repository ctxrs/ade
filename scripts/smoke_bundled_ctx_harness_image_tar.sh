#!/usr/bin/env bash
set -euo pipefail

# Smoke test for the *bundled* ctx-harness image tarball.
#
# Usage:
#   scripts/smoke_bundled_ctx_harness_image_tar.sh <bundle_dir>
#
# This loads the ctx-harness image tar from <bundle_dir>/manifest.json into the sandbox runtime and then runs the
# standard egress-guard smoke test against the loaded image reference.

bundle_dir="${1:-}"
if [[ -z "${bundle_dir// /}" ]]; then
  echo "usage: $0 <bundle_dir>" >&2
  exit 2
fi

if [[ ! -f "$bundle_dir/manifest.json" ]]; then
  echo "missing manifest: $bundle_dir/manifest.json" >&2
  exit 2
fi

arch_raw="$(uname -m || true)"
case "$arch_raw" in
  x86_64|amd64) arch="x86_64" ;;
  aarch64|arm64) arch="aarch64" ;;
  *) echo "unsupported host arch: $arch_raw" >&2; exit 2 ;;
esac

if ! command -v python3 >/dev/null 2>&1; then
  echo "missing python3" >&2
  exit 2
fi

readarray -t meta < <(python3 - "$bundle_dir/manifest.json" "$arch" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
arch = sys.argv[2]

manifest = json.loads(path.read_text(encoding="utf-8"))
images = manifest.get("images") or []
for img in images:
    if img.get("id") == "ctx-harness" and img.get("os") == "linux" and img.get("arch") == arch:
        tar = img.get("tar") or ""
        image = img.get("image") or ""
        if tar and image:
            print(tar)
            print(image)
            raise SystemExit(0)
print("")
print("")
raise SystemExit(1)
PY
)

tar_rel="${meta[0]:-}"
image_ref="${meta[1]:-}"
if [[ -z "${tar_rel// /}" || -z "${image_ref// /}" ]]; then
  echo "ctx-harness image entry not found in manifest for linux/$arch" >&2
  exit 2
fi

tar_path="$bundle_dir/$tar_rel"
if [[ ! -f "$tar_path" ]]; then
  echo "missing image tar: $tar_path" >&2
  exit 2
fi

runtime="${CONTAINER_RUNTIME:-nerdctl}"
if ! command -v "$runtime" >/dev/null 2>&1; then
  echo "missing container runtime: $runtime" >&2
  exit 2
fi

"$runtime" load -i "$tar_path" >/dev/null

CONTAINER_RUNTIME="$runtime" \
CTX_HARNESS_IMAGE="$image_ref" \
"$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/smoke_ctx_harness_egress_guard.sh"
