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

for cmd in find file readelf grep rm; do
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

while IFS= read -r -d '' path; do
  file_desc="$(file -b "$path" 2>/dev/null || true)"
  [[ "$file_desc" == *"ELF"* ]] || continue

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
  fi
done < <(find "$bundles_dir" -type f -print0)

echo "pruned_foreign_arch_elf=$pruned_foreign_arch_elf"
echo "pruned_musl_targeted_elf=$pruned_musl_targeted_elf"

# Enforce contract: no target-arch musl-linked ELF can remain for glibc packaging.
remaining_musl=0
while IFS= read -r -d '' path; do
  file_desc="$(file -b "$path" 2>/dev/null || true)"
  [[ "$file_desc" == *"ELF"* ]] || continue
  [[ "$file_desc" == *"$elf_arch_token"* ]] || continue
  if is_musl_linked_elf "$path"; then
    echo "error: musl-linked target-arch ELF remains after prune: $path [$file_desc]" >&2
    remaining_musl=1
  fi
done < <(find "$bundles_dir" -type f -print0)

if [[ "$remaining_musl" -ne 0 ]]; then
  echo "error: glibc release package still contains musl-linked target-arch ELF artifacts" >&2
  exit 1
fi
