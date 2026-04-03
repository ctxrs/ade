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

if ! ldconfig -p | grep -Fq "libc++.so.9.0"; then
  echo "error: required libc++ soname is not registered in linker cache: libc++.so.9.0" >&2
  echo "       release lane tool deps must register it before invoking this script." >&2
  exit 1
fi

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

pick_latest_bundle_artifact() {
  local search_dir="$1"
  local pattern="$2"
  if [[ ! -d "$search_dir" ]]; then
    return 0
  fi
  local latest
  latest="$(
    find "$search_dir" -maxdepth 1 -type f -name "$pattern" -printf '%T@ %p\n' 2>/dev/null \
      | LC_ALL=C sort -n \
      | tail -n 1 \
      | sed -E 's/^[0-9]+(\.[0-9]+)? //'
  )"
  printf '%s' "$latest"
}

pick_latest_bundle_artifact_from_candidates() {
  local pattern="$1"
  shift
  local search_dir latest
  for search_dir in "$@"; do
    latest="$(pick_latest_bundle_artifact "$search_dir" "$pattern")"
    if [[ -n "$latest" ]]; then
      printf '%s' "$latest"
      return 0
    fi
  done
  return 0
}

appimage_bundle_dir="core/apps/desktop/src-tauri/target/release/bundle/appimage"
fallback_appimage_bundle_dir="core/target/release/bundle/appimage"

if ! \
  CTX_DESKTOP_SYNC_BUNDLES=0 \
  CTX_BUNDLE_REMOTE_DAEMONS=0 \
  RUST_LOG=tauri_bundler=debug \
  pnpm -C core/apps/desktop run build -- --bundles appimage; then
  echo "::group::linuxdeploy diagnostics (${platform})"

  tauri_cache_dir="$(select_tauri_cache_dir)"
  linuxdeploy_path="$tauri_cache_dir/linuxdeploy-${tauri_arch}.AppImage"
  plugin_path="$tauri_cache_dir/linuxdeploy-plugin-appimage-${tauri_arch}.AppImage"
  if [[ ! -f "$plugin_path" ]]; then
    plugin_path="$tauri_cache_dir/linuxdeploy-plugin-appimage.AppImage"
  fi
  appdir_path="$(find "$appimage_bundle_dir" "$fallback_appimage_bundle_dir" -maxdepth 1 -type d -name '*.AppDir' 2>/dev/null | head -n 1 || true)"

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

  manual_linuxdeploy_ok=0
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
      if "$linuxdeploy_path" --appimage-extract-and-run --verbosity 3 --appdir "$appdir_path" --plugin gtk --output appimage; then
        manual_linuxdeploy_ok=1
      fi
    fi
  fi

  echo "::endgroup::"
  if [[ "$manual_linuxdeploy_ok" -eq 1 ]]; then
    mkdir -p "$appimage_bundle_dir"
    appimage_after_recovery="$(pick_latest_bundle_artifact "$appimage_bundle_dir" '*.AppImage')"
    if [[ -z "$appimage_after_recovery" ]]; then
      appimage_after_recovery="$(pick_latest_bundle_artifact "." '*.AppImage')"
    fi
    if [[ -n "$appimage_after_recovery" && -f "$appimage_after_recovery" ]]; then
      if [[ "$appimage_after_recovery" != "$appimage_bundle_dir/"* ]]; then
        recovered_basename="$(basename "$appimage_after_recovery")"
        normalized_recovery_path="$appimage_bundle_dir/$recovered_basename"
        mv -f "$appimage_after_recovery" "$normalized_recovery_path"
        appimage_after_recovery="$normalized_recovery_path"
      fi
      # EXCEPTION: we accept manual linuxdeploy recovery when the initial tauri bundler
      # invocation fails but deterministic artifact generation succeeds in the same lane.
      echo "recovered AppImage bundle after tauri linuxdeploy failure: $appimage_after_recovery"
    else
      echo "error: linuxdeploy diagnostics succeeded but no AppImage artifact was produced"
      exit 1
    fi
  else
    exit 1
  fi
fi

appimage_bundle="$(pick_latest_bundle_artifact_from_candidates '*.AppImage' "$appimage_bundle_dir" "$fallback_appimage_bundle_dir")"
if [[ -z "$appimage_bundle" || ! -f "$appimage_bundle" ]]; then
  echo "error: linux AppImage bundle missing after Tauri build"
  exit 1
fi
