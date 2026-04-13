#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
BUNDLE_SCRIPT="$ROOT/scripts/ensure_bundled_harnesses.sh"

if [[ ! -x "$BUNDLE_SCRIPT" ]]; then
  echo "error: missing bundle script: $BUNDLE_SCRIPT" >&2
  exit 2
fi

if ! command -v docker >/dev/null 2>&1; then
  echo "error: docker is required for this regression test" >&2
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

docker buildx version >/dev/null
docker info >/dev/null

tmp_root="$(mktemp -d /tmp/ctx-bundle-docker-only.XXXXXX)"
temp_builder="ctx-bundle-docker-only-$$"
original_builder="$(docker buildx ls 2>/dev/null | awk '$1 ~ /\*/ {gsub(/\*/, "", $1); print $1; exit}')"
cleanup() {
  if [[ -n "${original_builder:-}" ]]; then
    docker buildx use "$original_builder" >/dev/null 2>&1 || true
  fi
  docker buildx rm -f "$temp_builder" >/dev/null 2>&1 || true
  rm -rf "$tmp_root"
}
trap cleanup EXIT

shim_dir="$tmp_root/shims"
mkdir -p "$shim_dir"
cat >"$shim_dir/nerdctl" <<'SH'
#!/usr/bin/env bash
echo "unexpected nerdctl invocation" >&2
exit 97
SH
chmod +x "$shim_dir/nerdctl"

host_arch_raw="$(uname -m 2>/dev/null || true)"
case "$host_arch_raw" in
  x86_64|amd64) host_arch="x86_64" ;;
  aarch64|arm64) host_arch="aarch64" ;;
  *)
    echo "error: unsupported host arch: $host_arch_raw" >&2
    exit 2
    ;;
esac

echo "smoke: harness image bundling ignores sandbox runtime CLIs even when nerdctl is first on PATH"
bundle_dir_image="$tmp_root/bundle-image"
if docker buildx inspect default >/dev/null 2>&1; then
  if docker buildx use default >/dev/null 2>&1; then
    echo "info: forced buildx default builder to exercise docker-driver fallback handling" >&2
  else
    echo "warn: unable to force default buildx builder; continuing with current builder" >&2
  fi
fi
PATH="$shim_dir:$PATH" \
CTX_BUNDLE_DIR="$bundle_dir_image" \
CTX_BUNDLE_BUILDX_BUILDER="$temp_builder" \
CTX_BUNDLE_ONLY_PROVIDERS="does-not-exist" \
CTX_BUNDLE_SKIP_RUNTIMES=1 \
CTX_BUNDLE_INCLUDE_BRIDGE=0 \
CTX_BUNDLE_LOCAL_ADAPTERS=off \
CTX_BUNDLE_BUILD_LOCAL_ADAPTERS=0 \
CTX_BUNDLE_HARNESS_IMAGE=1 \
"$BUNDLE_SCRIPT"

image_tar="$bundle_dir_image/images/ctx-harness-linux-${host_arch}.tar"
if [[ ! -f "$image_tar" ]]; then
  echo "error: expected harness image tar missing: $image_tar" >&2
  exit 1
fi

if ! "$python_cmd" - "$bundle_dir_image/manifest.json" "$host_arch" <<'PY'
import json
import sys
from pathlib import Path

manifest_path = Path(sys.argv[1])
arch = sys.argv[2]
data = json.loads(manifest_path.read_text(encoding="utf-8"))
images = data.get("images") or []
for image in images:
    if image.get("id") == "ctx-harness" and image.get("os") == "linux" and image.get("arch") == arch:
        tar_rel = image.get("tar")
        if isinstance(tar_rel, str) and tar_rel.strip():
            raise SystemExit(0)
raise SystemExit(1)
PY
then
  echo "error: manifest missing ctx-harness linux/${host_arch} image entry" >&2
  exit 1
fi

echo "ok: ensure_bundled_harnesses build-time paths are docker-only"
