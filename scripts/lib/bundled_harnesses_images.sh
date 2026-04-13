# Harness image bundling and final manifest emission for bundled harness prep.

bundle_harness_image() {
  local image_ref="$1"
  local tar_rel="$2"
  local tar_path="$bundle_dir/$tar_rel"
  local platform="${3:-}"

  mkdir -p "$(dirname "$tar_path")"

  # Do not silently produce a mislabeled tar when cross-bundling (e.g. CTX_BUNDLE_ARCH=x86_64 on arm64).
  if [[ -z "$platform" ]]; then
    log "error: internal: missing platform for harness image bundle"
    exit 2
  fi

  if ! ensure_docker_ready_for_builds "1" "harness image bundling"; then
    exit 2
  fi

  local builder_name="${CTX_BUNDLE_BUILDX_BUILDER:-ctx-bundle-export}"
  if ! docker buildx inspect "$builder_name" >/dev/null 2>&1; then
    docker buildx create --name "$builder_name" --driver docker-container --use >/dev/null
  else
    docker buildx use "$builder_name" >/dev/null
  fi
  docker buildx inspect --bootstrap "$builder_name" >/dev/null

  # Build the requested Linux arch and write a docker-archive compatible tar.
  docker buildx build \
    --builder "$builder_name" \
    --progress plain \
    --platform "$platform" \
    -t "$image_ref" \
    -f "$ROOT/containers/ctx-harness/Dockerfile" \
    --output "type=docker,dest=$tar_path" \
    "$ROOT"
}

bundle_harness_manifest_entry() {
  local image_ref="$1"
  local image_arch="$2"
  local tar_rel="$3"
  local sha="$4"
  HARNESS_IMAGE_REF_ENV="$image_ref" \
  HARNESS_IMAGE_VERSION_ENV="$image_ref" \
  HARNESS_IMAGE_OS_ENV="linux" \
  HARNESS_IMAGE_ARCH_ENV="$image_arch" \
  HARNESS_IMAGE_SHA_ENV="$sha" \
  HARNESS_IMAGE_TAR_REL_ENV="$tar_rel" \
  run_python - <<'PY' >> "$images_out"
import json
import os

entry = {
    "id": "ctx-harness",
    "version": os.environ["HARNESS_IMAGE_VERSION_ENV"],
    "os": os.environ["HARNESS_IMAGE_OS_ENV"],
    "arch": os.environ["HARNESS_IMAGE_ARCH_ENV"],
    "sha256": os.environ["HARNESS_IMAGE_SHA_ENV"],
    "tar": os.environ["HARNESS_IMAGE_TAR_REL_ENV"],
    "image": os.environ["HARNESS_IMAGE_REF_ENV"],
}
print(json.dumps(entry, separators=(",", ":")))
PY
}

bundle_harness_mode="${CTX_BUNDLE_HARNESS_IMAGE:-}"
bundle_harness_mode="${bundle_harness_mode// /}"
if is_truthy "$skip_images_raw"; then
  bundle_harness_mode=""
fi
if is_truthy "${bundle_harness_mode:-}"; then
  HARNESS_IMAGE_REF="$(read_const DEFAULT_CONTAINER_IMAGE "$ROOT/core/crates/ctx-sandbox-container-runtime/src/lib.rs")"
  harness_tar_rel="images/ctx-harness-linux-${arch}.tar"
  harness_platform="linux/amd64"
  if [[ "$arch" == "aarch64" ]]; then
    harness_platform="linux/arm64"
  fi
  bundle_harness_image "$HARNESS_IMAGE_REF" "$harness_tar_rel" "$harness_platform"
  harness_sha="$(sha256_file "$bundle_dir/$harness_tar_rel")"
  bundle_harness_manifest_entry "$HARNESS_IMAGE_REF" "$arch" "$harness_tar_rel" "$harness_sha"
elif [[ "$bundle_harness_mode" == "both" || "$bundle_harness_mode" == "all" ]]; then
  HARNESS_IMAGE_REF="$(read_const DEFAULT_CONTAINER_IMAGE "$ROOT/core/crates/ctx-sandbox-container-runtime/src/lib.rs")"

  tar_x86="images/ctx-harness-linux-x86_64.tar"
  tar_arm="images/ctx-harness-linux-aarch64.tar"

  bundle_harness_image "$HARNESS_IMAGE_REF" "$tar_x86" "linux/amd64"
  sha_x86="$(sha256_file "$bundle_dir/$tar_x86")"
  bundle_harness_manifest_entry "$HARNESS_IMAGE_REF" "x86_64" "$tar_x86" "$sha_x86"

  bundle_harness_image "$HARNESS_IMAGE_REF" "$tar_arm" "linux/arm64"
  sha_arm="$(sha256_file "$bundle_dir/$tar_arm")"
  bundle_harness_manifest_entry "$HARNESS_IMAGE_REF" "aarch64" "$tar_arm" "$sha_arm"
fi

manifest_path="$bundle_dir/manifest.json"
GENERATED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
PROVIDERS_OUT="$providers_out" \
RUNTIMES_OUT="$runtimes_out" \
IMAGES_OUT="$images_out" \
GENERATED_AT_ENV="$GENERATED_AT" \
MANIFEST_PATH="$manifest_path" \
BUNDLE_APPEND_ENV="${CTX_BUNDLE_APPEND:-}" \
run_python - <<'PY'
import json
import os
from pathlib import Path

providers_path = Path(os.environ["PROVIDERS_OUT"])
runtimes_path = Path(os.environ["RUNTIMES_OUT"])
images_path = Path(os.environ["IMAGES_OUT"])
manifest_path = Path(os.environ.get("MANIFEST_PATH", "manifest.json"))

providers = []
if providers_path.exists():
    for line in providers_path.read_text().splitlines():
        if line.strip():
            providers.append(json.loads(line))

runtimes = []
if runtimes_path.exists():
    for line in runtimes_path.read_text().splitlines():
        if line.strip():
            runtimes.append(json.loads(line))

images = []
if images_path.exists():
    for line in images_path.read_text().splitlines():
        if line.strip():
            images.append(json.loads(line))

manifest = {
    "version": 1,
    "generated_at": os.environ["GENERATED_AT_ENV"],
    "providers": providers,
    "runtimes": runtimes,
    "images": images,
}

append = os.environ.get("BUNDLE_APPEND_ENV", "").strip().lower() in ("1", "true", "yes")
if append and manifest_path.exists():
    try:
        existing = json.loads(manifest_path.read_text())
    except Exception:
        existing = None
    if isinstance(existing, dict) and existing.get("version") == manifest["version"]:
        providers_by_key = {}
        for entry in existing.get("providers", []) or []:
            key = (
                entry.get("id"),
                entry.get("protocol"),
                entry.get("os"),
                entry.get("arch"),
            )
            providers_by_key[key] = entry
        for entry in providers:
            key = (
                entry.get("id"),
                entry.get("protocol"),
                entry.get("os"),
                entry.get("arch"),
            )
            providers_by_key[key] = entry
        runtimes_by_key = {}
        for entry in existing.get("runtimes", []) or []:
            key = (entry.get("id"), entry.get("os"), entry.get("arch"))
            runtimes_by_key[key] = entry
        for entry in runtimes:
            key = (entry.get("id"), entry.get("os"), entry.get("arch"))
            runtimes_by_key[key] = entry
        manifest["providers"] = sorted(
            providers_by_key.values(),
            key=lambda e: (
                e.get("id", ""),
                e.get("protocol", ""),
                e.get("os", ""),
                e.get("arch", ""),
            ),
        )
        manifest["runtimes"] = sorted(
            runtimes_by_key.values(),
            key=lambda e: (e.get("id", ""), e.get("os", ""), e.get("arch", "")),
        )
        images_by_key = {}
        for entry in existing.get("images", []) or []:
            key = (entry.get("id"), entry.get("os"), entry.get("arch"))
            images_by_key[key] = entry
        for entry in images:
            key = (entry.get("id"), entry.get("os"), entry.get("arch"))
            images_by_key[key] = entry
        manifest["images"] = sorted(
            images_by_key.values(),
            key=lambda e: (e.get("id", ""), e.get("os", ""), e.get("arch", "")),
        )

manifest_path.write_text(json.dumps(manifest, indent=2))
PY

rm -f "$providers_src" "$providers_out" "$runtimes_out" "$local_providers_src" || true

echo "$bundle_dir"
