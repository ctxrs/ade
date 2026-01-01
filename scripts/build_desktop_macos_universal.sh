#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "error: this script must run on macOS (Darwin)" >&2
  exit 2
fi

if ! command -v lipo >/dev/null 2>&1; then
  echo "error: missing 'lipo' (install Xcode Command Line Tools)" >&2
  exit 2
fi

ARM_TARGET="aarch64-apple-darwin"
INTEL_TARGET="x86_64-apple-darwin"

VERSION="$(node -p "require('./core/apps/desktop/package.json').version")"
PRODUCT_NAME="$(node -p "require('./core/apps/desktop/src-tauri/tauri.conf.json').package.productName")"

echo "Building macOS universal desktop ($PRODUCT_NAME $VERSION)..."

echo "Ensuring Rust targets exist..."
rustup target add "$ARM_TARGET" "$INTEL_TARGET" >/dev/null

echo "Checking versions..."
node core/scripts/desktop_check_versions.cjs >/dev/null

echo "Building web assets..."
pnpm -C core/apps/web build >/dev/null

echo "Building sidecars (both arch)..."
# The Rust workspace lives under `core/` (this script runs from the repo root).
cargo build --manifest-path core/Cargo.toml -p ctx-http -p ctx-mcp --release --target "$ARM_TARGET"
cargo build --manifest-path core/Cargo.toml -p ctx-http -p ctx-mcp --release --target "$INTEL_TARGET"

echo "Syncing universal sidecars + web dist into desktop resources..."
node core/scripts/desktop_sync_resources_universal_macos.cjs --profile release

echo "Building per-arch Tauri bundles..."
# Use `pnpm exec` so `--target` is passed to the Tauri CLI (not as Cargo args after `--`).
pnpm -C core/apps/desktop exec tauri build --target "$ARM_TARGET"
pnpm -C core/apps/desktop exec tauri build --target "$INTEL_TARGET"

ARM_APP="core/apps/desktop/src-tauri/target/$ARM_TARGET/release/bundle/macos/$PRODUCT_NAME.app"
INTEL_APP="core/apps/desktop/src-tauri/target/$INTEL_TARGET/release/bundle/macos/$PRODUCT_NAME.app"

if [[ ! -d "$ARM_APP" ]]; then
  echo "error: missing arm app at $ARM_APP" >&2
  exit 3
fi
if [[ ! -d "$INTEL_APP" ]]; then
  echo "error: missing intel app at $INTEL_APP" >&2
  exit 3
fi

OUT_ROOT="core/apps/desktop/src-tauri/target/universal"
OUT_APP="$OUT_ROOT/release/bundle/macos/$PRODUCT_NAME.app"
OUT_DMG_DIR="$OUT_ROOT/release/bundle/dmg"
OUT_ZIP_DIR="$OUT_ROOT/release/bundle/zip"
mkdir -p "$(dirname "$OUT_APP")" "$OUT_DMG_DIR" "$OUT_ZIP_DIR"

echo "Creating universal app at $OUT_APP..."
rm -rf "$OUT_APP"
cp -R "$ARM_APP" "$OUT_APP"

merge_machos_in_dir() {
  local rel_dir="$1"
  local arm_dir="$ARM_APP/$rel_dir"
  local intel_dir="$INTEL_APP/$rel_dir"
  local out_dir="$OUT_APP/$rel_dir"

  if [[ ! -d "$arm_dir" || ! -d "$intel_dir" || ! -d "$out_dir" ]]; then
    return 0
  fi

  while IFS= read -r -d '' arm_file; do
    local rel="${arm_file#$arm_dir/}"
    local intel_file="$intel_dir/$rel"
    local out_file="$out_dir/$rel"
    if [[ ! -f "$intel_file" || ! -f "$out_file" ]]; then
      continue
    fi

    # Only lipo Mach-O binaries.
    if ! file "$arm_file" | grep -q "Mach-O"; then
      continue
    fi
    if ! file "$intel_file" | grep -q "Mach-O"; then
      continue
    fi

    # `lipo -create` fails if inputs contain the same arch. Extract only the
    # missing slices from the intel binary and merge those into the arm binary.
    local -a arm_archs
    local -a intel_archs
    read -r -a arm_archs <<<"$(lipo -archs "$arm_file" 2>/dev/null || true)"
    read -r -a intel_archs <<<"$(lipo -archs "$intel_file" 2>/dev/null || true)"
    if [[ ${#arm_archs[@]} -eq 0 || ${#intel_archs[@]} -eq 0 ]]; then
      continue
    fi

    has_arch() {
      local needle="$1"
      shift
      local arch
      for arch in "$@"; do
        if [[ "$arch" == "$needle" ]]; then
          return 0
        fi
      done
      return 1
    }

    local -a missing
    local arch
    for arch in "${intel_archs[@]}"; do
      if ! has_arch "$arch" "${arm_archs[@]}"; then
        missing+=("$arch")
      fi
    done
    if [[ ${#missing[@]} -eq 0 ]]; then
      continue
    fi

    local -a inputs
    local -a tmp_files
    inputs=("$arm_file")
    tmp_files=()

    if [[ ${#intel_archs[@]} -eq 1 ]]; then
      # Thin binaries can't be `lipo -extract`'d; use the file directly.
      inputs+=("$intel_file")
    else
      local tmp
      for arch in "${missing[@]}"; do
        tmp="$(mktemp "/tmp/ctx-lipo.${arch}.XXXXXX")"
        tmp_files+=("$tmp")
        inputs+=("$tmp")
        lipo -extract "$arch" "$intel_file" -output "$tmp"
      done
    fi

    lipo -create "${inputs[@]}" -output "$out_file"
    if [[ ${#tmp_files[@]} -gt 0 ]]; then
      rm -f "${tmp_files[@]}"
    fi
  done < <(find "$arm_dir" -type f -print0)
}

copy_missing_files_in_dir() {
  local rel_dir="$1"
  local src_dir="$INTEL_APP/$rel_dir"
  local out_dir="$OUT_APP/$rel_dir"

  if [[ ! -d "$src_dir" || ! -d "$out_dir" ]]; then
    return 0
  fi

  while IFS= read -r -d '' src_file; do
    local rel="${src_file#$src_dir/}"
    local out_file="$out_dir/$rel"
    if [[ -f "$out_file" ]]; then
      continue
    fi
    mkdir -p "$(dirname "$out_file")"
    cp "$src_file" "$out_file"
  done < <(find "$src_dir" -type f -print0)
}

# Main app binary/binaries.
merge_machos_in_dir "Contents/MacOS"

# Sidecars and other embedded binaries packaged as resources.
merge_machos_in_dir "Contents/Resources/bin"
copy_missing_files_in_dir "Contents/Resources/bin"
copy_missing_files_in_dir "Contents/Resources"

echo "Building a simple universal DMG..."
STAGE="$(mktemp -d /tmp/ctx-universal-dmg-stage.XXXXXX)"
trap 'rm -rf "$STAGE"' EXIT
cp -R "$OUT_APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"

DMG_OUT="$OUT_DMG_DIR/ctx_${VERSION}_macos_universal.dmg"
rm -f "$DMG_OUT"
hdiutil create -volname "$PRODUCT_NAME" -srcfolder "$STAGE" -ov -format UDZO "$DMG_OUT" >/dev/null

echo "Building a notarization-friendly zip of the .app..."
ZIP_OUT="$OUT_ZIP_DIR/ctx_${VERSION}_macos_universal.zip"
rm -f "$ZIP_OUT"
ditto -c -k --sequesterRsrc --keepParent "$OUT_APP" "$ZIP_OUT"

echo "Done:"
echo "- $DMG_OUT"
echo "- $ZIP_OUT"
