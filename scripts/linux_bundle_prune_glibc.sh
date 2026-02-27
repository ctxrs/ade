#!/usr/bin/env bash
set -euo pipefail

bundles_dir="${BUNDLES_DIR:-core/apps/desktop/src-tauri/bundles}"
release_platform="${RELEASE_PLATFORM:-}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --bundles-dir)
      bundles_dir="${2:-}"
      shift 2
      ;;
    --platform)
      release_platform="${2:-}"
      shift 2
      ;;
    *)
      echo "error: unsupported argument: $1" >&2
      exit 1
      ;;
  esac
done

if [[ ! -d "$bundles_dir" ]]; then
  echo "error: bundles directory not found: $bundles_dir" >&2
  exit 1
fi

if [[ -z "$release_platform" ]]; then
  case "$(uname -m)" in
    x86_64|amd64)
      release_platform="linux-x64"
      ;;
    aarch64|arm64)
      release_platform="linux-arm64"
      ;;
    *)
      echo "error: unable to infer linux platform from uname -m: $(uname -m)" >&2
      exit 1
      ;;
  esac
fi

case "$release_platform" in
  linux-x64)
    elf_arch_token="x86-64"
    ;;
  linux-arm64)
    elf_arch_token="ARM aarch64"
    ;;
  *)
    echo "error: unsupported linux release platform: $release_platform" >&2
    exit 1
    ;;
esac

for cmd in awk cksum chmod find file grep gzip mkdir mv readelf rm; do
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "error: required tool '$cmd' is missing from PATH" >&2
    exit 1
  fi
done

is_musl_linked_elf() {
  local path="$1"
  local program_headers dynamic_headers
  program_headers="$(readelf -l "$path" 2>/dev/null || true)"
  if printf '%s\n' "$program_headers" | grep -Eq 'Requesting program interpreter: .*musl'; then
    return 0
  fi

  dynamic_headers="$(readelf -d "$path" 2>/dev/null || true)"
  if [[ -z "$dynamic_headers" ]]; then
    return 1
  fi

  # Most explicit signal for musl-linked ELF.
  if printf '%s\n' "$dynamic_headers" | grep -Eq 'Shared library: \[libc\.musl-[^]]+\.so\.1\]'; then
    return 0
  fi

  # Many musl provider sidecars expose unversioned libc.so (not libc.so.6), which
  # fails glibc AppImage packaging in linuxdeploy/ldd.
  if printf '%s\n' "$dynamic_headers" | grep -Eq 'Shared library: \[libc\.so\]'; then
    return 0
  fi

  return 1
}

pruned_foreign_arch_elf=0
pruned_musl_targeted_elf=0
pruned_non_linux_os_elf=0
wrapped_static_provider_elf=0

is_non_linux_path_elf() {
  local path="$1"
  local lower
  lower="$(printf '%s' "$path" | tr '[:upper:]' '[:lower:]')"
  if [[ "$lower" == *"/freebsd"* || "$lower" == *"/darwin"* || "$lower" == *"/macos"* || "$lower" == *"/win32"* || "$lower" == *"/windows"* || "$lower" == *"/openbsd"* || "$lower" == *"/netbsd"* || "$lower" == *"/android"* || "$lower" == *"/sunos"* || "$lower" == *"/aix"* ]]; then
    return 0
  fi
  return 1
}

is_provider_bundle_path() {
  local path="$1"
  local lower
  lower="$(printf '%s' "$path" | tr '[:upper:]' '[:lower:]')"
  if [[ "$lower" == *"/providers/"* ]]; then
    return 0
  fi
  return 1
}

is_static_elf() {
  local path="$1"
  local dynamic_headers
  dynamic_headers="$(readelf -d "$path" 2>/dev/null || true)"
  if [[ -z "$dynamic_headers" ]]; then
    return 0
  fi
  if printf '%s\n' "$dynamic_headers" | grep -Eq 'Shared library: \['; then
    return 1
  fi
  return 0
}

wrap_static_provider_elf() {
  local path="$1"
  local payload="${path}.ctxbin.gz"

  if [[ -f "$payload" ]]; then
    rm -f "$payload"
  fi

  gzip -n -f "$path"
  mv -f "${path}.gz" "$payload"

  cat > "$path" <<'SH'
#!/bin/sh
set -eu

self_path="$0"
payload="${self_path}.ctxbin.gz"
if [ ! -f "$payload" ]; then
  echo "error: missing static payload: $payload" >&2
  exit 127
fi

cache_root="${XDG_CACHE_HOME:-$HOME/.cache}/ctx/static-provider-bin"
mkdir -p "$cache_root"
payload_sig="$(cksum "$payload" | awk '{print $1"-"$2}')"
cache_bin="$cache_root/$(basename "$self_path").$payload_sig"
if [ ! -x "$cache_bin" ]; then
  tmp="$cache_bin.tmp.$$"
  gzip -dc "$payload" > "$tmp"
  chmod 0755 "$tmp"
  mv -f "$tmp" "$cache_bin"
fi

exec "$cache_bin" "$@"
SH
  chmod 0755 "$path"
}

while IFS= read -r -d '' path; do
  file_desc="$(file -b "$path" 2>/dev/null || true)"
  [[ "$file_desc" == *"ELF"* ]] || continue

  if is_non_linux_path_elf "$path"; then
    echo "pruning non-linux target-path ELF from glibc package: $path [$file_desc]"
    rm -f "$path"
    pruned_non_linux_os_elf=$((pruned_non_linux_os_elf + 1))
    continue
  fi

  if [[ "$file_desc" != *"$elf_arch_token"* ]]; then
    echo "pruning foreign-arch ELF: $path [$file_desc]"
    rm -f "$path"
    pruned_foreign_arch_elf=$((pruned_foreign_arch_elf + 1))
    continue
  fi

  if is_musl_linked_elf "$path"; then
    echo "pruning musl-linked ELF from glibc package: $path [$file_desc]"
    rm -f "$path"
    pruned_musl_targeted_elf=$((pruned_musl_targeted_elf + 1))
    continue
  fi

  # linuxdeploy GTK plugin aborts on static ELF payloads. Preserve provider runtime paths by
  # replacing static provider ELF files with a launcher script + compressed payload.
  if is_provider_bundle_path "$path" && is_static_elf "$path"; then
    echo "wrapping static provider ELF for glibc package: $path [$file_desc]"
    wrap_static_provider_elf "$path"
    wrapped_static_provider_elf=$((wrapped_static_provider_elf + 1))
  fi
done < <(find "$bundles_dir" -type f -print0)

echo "pruned_foreign_arch_elf=$pruned_foreign_arch_elf"
echo "pruned_musl_targeted_elf=$pruned_musl_targeted_elf"
echo "pruned_non_linux_os_elf=$pruned_non_linux_os_elf"
echo "wrapped_static_provider_elf=$wrapped_static_provider_elf"

# Enforce contract: no target-arch musl-linked ELF can remain for glibc packaging.
remaining_musl=0
remaining_non_linux=0
remaining_static_provider=0
while IFS= read -r -d '' path; do
  file_desc="$(file -b "$path" 2>/dev/null || true)"
  [[ "$file_desc" == *"ELF"* ]] || continue
  [[ "$file_desc" == *"$elf_arch_token"* ]] || continue
  if is_non_linux_path_elf "$path"; then
    echo "error: non-linux target-path ELF remains after prune: $path [$file_desc]" >&2
    remaining_non_linux=1
    continue
  fi
  if is_musl_linked_elf "$path"; then
    echo "error: musl-linked target-arch ELF remains after prune: $path [$file_desc]" >&2
    remaining_musl=1
    continue
  fi
  if is_provider_bundle_path "$path" && is_static_elf "$path"; then
    echo "error: static provider ELF remains after prune/wrap: $path [$file_desc]" >&2
    remaining_static_provider=1
  fi
done < <(find "$bundles_dir" -type f -print0)

if [[ "$remaining_musl" -ne 0 ]]; then
  echo "error: glibc release package still contains musl-linked target-arch ELF artifacts" >&2
  exit 1
fi

if [[ "$remaining_non_linux" -ne 0 ]]; then
  echo "error: glibc release package still contains non-linux target-path ELF artifacts" >&2
  exit 1
fi

if [[ "$remaining_static_provider" -ne 0 ]]; then
  echo "error: glibc release package still contains static provider ELF artifacts" >&2
  exit 1
fi
