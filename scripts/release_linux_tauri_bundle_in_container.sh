#!/usr/bin/env bash
set -euo pipefail

platform="${RELEASE_PLATFORM:-}"
case "$platform" in
  linux-x64)
    tauri_arch="x86_64"
    musl_loader_name="ld-musl-x86_64.so.1"
    musl_soname="libc.musl-x86_64.so.1"
    ;;
  linux-arm64)
    tauri_arch="aarch64"
    musl_loader_name="ld-musl-aarch64.so.1"
    musl_soname="libc.musl-aarch64.so.1"
    ;;
  *)
    echo "error: unsupported RELEASE_PLATFORM for linux containerized Tauri build: ${platform:-<empty>}" >&2
    exit 1
    ;;
esac

for cmd in pnpm patchelf file readelf ldd appstreamcli xdg-mime desktop-file-validate mksquashfs zsyncmake gtk-update-icon-cache; do
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "error: missing required command in release image: $cmd" >&2
    exit 1
  fi
done

scripts/linux_bundle_gate.sh --platform "$platform" --mode both

if ! ldconfig -p | grep -Fq "$musl_soname"; then
  echo "error: required musl soname is not registered in linker cache: $musl_soname" >&2
  echo "       release lane tool deps must register it before invoking this script." >&2
  exit 1
fi

# linuxdeploy must resolve hashed bundled glibc library names during AppImage packaging.
# Keep this scoped to non-musl Linux bundle directories to avoid reintroducing musl/glibc conflicts.
bundle_lib_path="$(
  find core/apps/desktop/src-tauri/bundles \
    -type f \( -name '*.so' -o -name '*.so.*' \) \
    -path "*/linux/${tauri_arch}/*" \
    ! -path '*musl*' \
    -print \
    | sed -E 's#/[^/]+$##' \
    | LC_ALL=C sort -u \
    | paste -sd: -
)"
if [[ -n "$bundle_lib_path" ]]; then
  if [[ -n "${LD_LIBRARY_PATH:-}" ]]; then
    export LD_LIBRARY_PATH="$bundle_lib_path:$LD_LIBRARY_PATH"
  else
    export LD_LIBRARY_PATH="$bundle_lib_path"
  fi
fi

export APPIMAGE_EXTRACT_AND_RUN=1

tauri_cache_candidates=()
if [[ -n "${XDG_CACHE_HOME:-}" ]]; then
  tauri_cache_candidates+=("$XDG_CACHE_HOME/tauri")
fi
if [[ -n "${HOME:-}" ]]; then
  tauri_cache_candidates+=("$HOME/.cache/tauri")
fi

select_tauri_cache_dir() {
  local linuxdeploy_name="linuxdeploy-${tauri_arch}.AppImage"
  local plugin_name="linuxdeploy-plugin-appimage-${tauri_arch}.AppImage"
  local candidate
  for candidate in "${tauri_cache_candidates[@]}"; do
    if [[ -f "$candidate/$linuxdeploy_name" || -f "$candidate/$plugin_name" ]]; then
      printf '%s' "$candidate"
      return 0
    fi
  done
  if [[ ${#tauri_cache_candidates[@]} -gt 0 ]]; then
    printf '%s' "${tauri_cache_candidates[0]}"
    return 0
  fi
  printf '%s' "."
}

if ! RUST_LOG=tauri_bundler=debug pnpm -C core/apps/desktop exec tauri build --bundles appimage; then
  echo "::group::linuxdeploy diagnostics (${platform})"

  tauri_cache_dir="$(select_tauri_cache_dir)"
  linuxdeploy_path="$tauri_cache_dir/linuxdeploy-${tauri_arch}.AppImage"
  plugin_path="$tauri_cache_dir/linuxdeploy-plugin-appimage-${tauri_arch}.AppImage"
  if [[ ! -f "$plugin_path" ]]; then
    plugin_path="$tauri_cache_dir/linuxdeploy-plugin-appimage.AppImage"
  fi
  appdir_path="$(find core/apps/desktop/src-tauri/target/release/bundle/appimage -maxdepth 1 -type d -name '*.AppDir' | head -n 1 || true)"

  echo "tauri_cache_candidates=${tauri_cache_candidates[*]:-<none>}"
  echo "tauri_cache_dir=$tauri_cache_dir"
  echo "musl_soname=$musl_soname"
  echo "musl_loader_name=${musl_loader_name}"
  echo "linuxdeploy_path=$linuxdeploy_path"
  echo "plugin_path=$plugin_path"
  echo "appdir_path=$appdir_path"

  for candidate in "${tauri_cache_candidates[@]}"; do
    echo "cache candidate listing: $candidate"
    ls -al "$candidate" || true
  done
  if [[ ! " ${tauri_cache_candidates[*]} " =~ " $tauri_cache_dir " ]]; then
    ls -al "$tauri_cache_dir" || true
  fi
  file "$linuxdeploy_path" "$plugin_path" || true
  chmod +x "$linuxdeploy_path" "$plugin_path" || true

  if [[ -x "$linuxdeploy_path" ]]; then
    export LINUXDEPLOY_PLUGIN_DIR="$tauri_cache_dir"
    "$linuxdeploy_path" --appimage-extract-and-run --version || true
  else
    echo "linuxdeploy binary is unavailable/executable at: $linuxdeploy_path"
  fi

  if [[ -n "$appdir_path" && -d "$appdir_path" ]]; then
    echo "::group::ldd probe (${platform})"
    ldd_probe_failed=0
    while IFS= read -r candidate; do
      file_desc="$(file -b "$candidate" 2>/dev/null || true)"
      if [[ "$file_desc" != *"ELF"* ]]; then
        continue
      fi
      if ! ldd "$candidate" >"/tmp/ldd-probe.out" 2>&1; then
        if grep -Eqi 'not a dynamic executable|statically linked' /tmp/ldd-probe.out; then
          continue
        fi
        echo "ldd probe failed for: $candidate"
        echo "file: $file_desc"
        cat /tmp/ldd-probe.out || true
        ldd_probe_failed=1
        continue
      fi
    done < <(find "$appdir_path" -type f | LC_ALL=C sort)
    if [[ "$ldd_probe_failed" -eq 0 ]]; then
      echo "ldd probe found no failing ELF files in $appdir_path"
    fi
    echo "::endgroup::"
    if [[ -x "$linuxdeploy_path" ]]; then
      "$linuxdeploy_path" --appimage-extract-and-run --verbosity 3 --appdir "$appdir_path" --plugin gtk --output appimage || true
    fi
  fi

  echo "::endgroup::"
  exit 1
fi
