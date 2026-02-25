#!/usr/bin/env bash
set -euo pipefail

mode="both"
bundles_dir="${BUNDLES_DIR:-core/apps/desktop/src-tauri/bundles}"
release_platform="${RELEASE_PLATFORM:-}"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --mode)
      mode="${2:-}"
      shift 2
      ;;
    --bundles-dir)
      bundles_dir="${2:-}"
      shift 2
      ;;
    --platform)
      release_platform="${2:-}"
      shift 2
      ;;
    *)
      echo "error: unsupported argument: $1"
      exit 1
      ;;
  esac
done

case "$mode" in
  normalize|closure|both) ;;
  *)
    echo "error: unsupported mode '$mode' (expected normalize|closure|both)"
    exit 1
    ;;
esac

if [[ ! -d "$bundles_dir" ]]; then
  echo "error: bundles directory not found: $bundles_dir"
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
      echo "error: unable to infer linux platform from uname -m: $(uname -m)"
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
    echo "error: unsupported linux release platform: $release_platform"
    exit 1
    ;;
esac

required_tools=(find file readelf ldd grep)
if [[ "$mode" == "normalize" || "$mode" == "both" ]]; then
  required_tools+=(patchelf)
fi

for cmd in "${required_tools[@]}"; do
  if ! command -v "$cmd" >/dev/null 2>&1; then
    echo "error: required tool '$cmd' is missing from PATH"
    exit 1
  fi
done

candidate_stream() {
  find "$bundles_dir" -type f \
    \( -name '*.so' -o -name '*.so.*' -o -perm -u+x -o -perm -g+x -o -perm -o+x \) \
    -print0
}

if [[ "$mode" == "normalize" || "$mode" == "both" ]]; then
  patched=0
  while IFS= read -r -d '' file_path; do
    file_desc="$(file -b "$file_path" 2>/dev/null || true)"
    [[ "$file_desc" == *"ELF"* ]] || continue
    [[ "$file_desc" == *"$elf_arch_token"* ]] || continue
    if readelf -d "$file_path" 2>/dev/null | grep -Eq 'libc\+\+\.so\.9\.0'; then
      patchelf --replace-needed libc++.so.9.0 libc++.so.1 "$file_path"
      patched=$((patched + 1))
      echo "patched libc++ soname: $file_path"
    fi
  done < <(candidate_stream)
  echo "patched_count=$patched"
fi

if [[ "$mode" == "closure" || "$mode" == "both" ]]; then
  unresolved=0
  while IFS= read -r -d '' file_path; do
    file_desc="$(file -b "$file_path" 2>/dev/null || true)"
    [[ "$file_desc" == *"ELF"* ]] || continue
    [[ "$file_desc" == *"$elf_arch_token"* ]] || continue
    ldd_out="$(ldd "$file_path" 2>&1 || true)"
    if printf '%s\n' "$ldd_out" | grep -Eiq 'not found'; then
      unresolved=1
      echo "::group::unresolved deps: $file_path"
      echo "$file_desc"
      printf '%s\n' "$ldd_out"
      echo "::endgroup::"
    fi
  done < <(candidate_stream)

  if [[ "$unresolved" -ne 0 ]]; then
    echo "error: unresolved shared-library dependencies found in bundled host-arch ELF artifacts"
    exit 1
  fi
fi
