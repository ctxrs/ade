#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PRUNE_SCRIPT="$ROOT/scripts/linux_bundle_prune_glibc.sh"

if [[ ! -x "$PRUNE_SCRIPT" ]]; then
  echo "error: missing prune script: $PRUNE_SCRIPT" >&2
  exit 2
fi

tmp_root="$(mktemp -d /tmp/ctx-linux-glibc-prune.XXXXXX)"
trap 'rm -rf "$tmp_root"' EXIT

bundles_dir="$tmp_root/bundles"
mock_bin="$tmp_root/mock-bin"
mkdir -p "$bundles_dir/providers/demo/linux/aarch64/lib" "$bundles_dir/providers/demo/linux/aarch64/bin" "$mock_bin"
mkdir -p "$bundles_dir/providers/demo/linux/aarch64/node_modules/koffi/build/koffi/freebsd_arm64"
mkdir -p "$bundles_dir/providers/demo/linux/aarch64/node_modules/@vendor/ripgrep/arm64-linux"
mkdir -p "$bundles_dir/providers/demo/linux/aarch64/bin/core-static"

glibc_ok="$bundles_dir/providers/demo/linux/aarch64/lib/glibc-ok.so"
musl_hidden="$bundles_dir/providers/demo/linux/aarch64/lib/libvips-cpp.so.42"
foreign_elf="$bundles_dir/providers/demo/linux/aarch64/bin/foreign-helper"
non_linux_os_elf="$bundles_dir/providers/demo/linux/aarch64/node_modules/koffi/build/koffi/freebsd_arm64/koffi.node"
static_node_elf="$bundles_dir/providers/demo/linux/aarch64/node_modules/@vendor/ripgrep/arm64-linux/rg"
static_core_elf="$bundles_dir/providers/demo/linux/aarch64/bin/core-static/provider-helper"
non_elf="$bundles_dir/providers/demo/linux/aarch64/lib/readme.txt"

printf 'glibc\n' > "$glibc_ok"
printf 'musl\n' > "$musl_hidden"
printf 'foreign\n' > "$foreign_elf"
printf 'freebsd\n' > "$non_linux_os_elf"
printf 'static-node\n' > "$static_node_elf"
printf 'static-core\n' > "$static_core_elf"
printf 'text\n' > "$non_elf"

cat > "$mock_bin/file" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
if [[ "${1:-}" == "-b" ]]; then
  shift
fi
target="${1:-}"
if [[ -f "$target" ]]; then
  first_line="$(head -n 1 "$target" 2>/dev/null || true)"
  if [[ "$first_line" == "#!/bin/sh" ]]; then
    echo "POSIX shell script, ASCII text executable"
    exit 0
  fi
fi
if [[ "$target" == *.ctxbin.gz ]]; then
  echo "gzip compressed data"
  exit 0
fi
case "$target" in
  *glibc-ok.so)
    echo "ELF 64-bit LSB shared object, ARM aarch64, version 1 (SYSV), dynamically linked"
    ;;
  *libvips-cpp.so.42)
    echo "ELF 64-bit LSB shared object, ARM aarch64, version 1 (SYSV), dynamically linked"
    ;;
  *foreign-helper)
    echo "ELF 64-bit LSB executable, x86-64, version 1 (SYSV), dynamically linked"
    ;;
  *freebsd_arm64/koffi.node)
    echo "ELF 64-bit LSB shared object, ARM aarch64, version 1 (SYSV), dynamically linked"
    ;;
  *arm64-linux/rg)
    echo "ELF 64-bit LSB executable, ARM aarch64, version 1 (SYSV), statically linked, stripped"
    ;;
  *core-static/provider-helper)
    echo "ELF 64-bit LSB executable, ARM aarch64, version 1 (SYSV), statically linked, stripped"
    ;;
  *)
    echo "ASCII text"
    ;;
esac
SH
chmod +x "$mock_bin/file"

cat > "$mock_bin/readelf" <<'SH'
#!/usr/bin/env bash
set -euo pipefail
mode="${1:-}"
target="${@: -1}"
case "$mode" in
  -l)
    case "$target" in
      *glibc-ok.so)
        cat <<'OUT'
Program Headers:
  INTERP         0x000000 0x000000 0x000000 0x000000 0x000000 R   0x1
      [Requesting program interpreter: /lib/ld-linux-aarch64.so.1]
OUT
        ;;
      *libvips-cpp.so.42)
        cat <<'OUT'
Program Headers:
OUT
        ;;
      *foreign-helper)
        cat <<'OUT'
Program Headers:
  INTERP         0x000000 0x000000 0x000000 0x000000 0x000000 R   0x1
      [Requesting program interpreter: /lib64/ld-linux-x86-64.so.2]
OUT
        ;;
      *)
        cat <<'OUT'
Program Headers:
OUT
        ;;
    esac
    ;;
  -d)
    case "$target" in
      *glibc-ok.so)
        cat <<'OUT'
Dynamic section at offset 0x0 contains 1 entry:
 0x0000000000000001 (NEEDED)             Shared library: [libc.so.6]
OUT
        ;;
      *libvips-cpp.so.42)
        cat <<'OUT'
Dynamic section at offset 0x0 contains 1 entry:
 0x0000000000000001 (NEEDED)             Shared library: [libc.so]
OUT
        ;;
      *foreign-helper)
        cat <<'OUT'
Dynamic section at offset 0x0 contains 1 entry:
 0x0000000000000001 (NEEDED)             Shared library: [libc.so.6]
OUT
        ;;
      *freebsd_arm64/koffi.node)
        cat <<'OUT'
Dynamic section at offset 0x0 contains 1 entry:
 0x0000000000000001 (NEEDED)             Shared library: [libc.so.6]
OUT
        ;;
      *arm64-linux/rg)
        cat <<'OUT'
Dynamic section at offset 0x0 contains 0 entries:
OUT
        ;;
      *core-static/provider-helper)
        cat <<'OUT'
Dynamic section at offset 0x0 contains 0 entries:
OUT
        ;;
      *)
        cat <<'OUT'
Dynamic section at offset 0x0 contains 0 entries:
OUT
        ;;
    esac
    ;;
  *)
    exit 0
    ;;
esac
SH
chmod +x "$mock_bin/readelf"

PATH="$mock_bin:$PATH" "$PRUNE_SCRIPT" --platform linux-arm64 --bundles-dir "$bundles_dir"

if [[ ! -f "$glibc_ok" ]]; then
  echo "error: expected glibc ELF to remain: $glibc_ok" >&2
  exit 1
fi
if [[ -f "$musl_hidden" ]]; then
  echo "error: expected musl-linked ELF to be pruned: $musl_hidden" >&2
  exit 1
fi
if [[ -f "$foreign_elf" ]]; then
  echo "error: expected foreign-arch ELF to be pruned: $foreign_elf" >&2
  exit 1
fi
if [[ -f "$non_linux_os_elf" ]]; then
  echo "error: expected non-linux target-path ELF to be pruned: $non_linux_os_elf" >&2
  exit 1
fi
if [[ -f "$static_node_elf" ]]; then
  if [[ ! -f "${static_node_elf}.ctxbin.gz" ]]; then
    echo "error: expected wrapped payload for static provider node/vendor ELF: ${static_node_elf}.ctxbin.gz" >&2
    exit 1
  fi
  if ! grep -q 'static-provider-bin' "$static_node_elf"; then
    echo "error: expected static provider node/vendor ELF to be replaced with launcher script: $static_node_elf" >&2
    exit 1
  fi
else
  echo "error: expected wrapped launcher to remain at static provider node/vendor path: $static_node_elf" >&2
  exit 1
fi
if [[ -f "$static_core_elf" ]]; then
  if [[ ! -f "${static_core_elf}.ctxbin.gz" ]]; then
    echo "error: expected wrapped payload for static provider ELF: ${static_core_elf}.ctxbin.gz" >&2
    exit 1
  fi
  if ! grep -q 'static-provider-bin' "$static_core_elf"; then
    echo "error: expected static provider ELF to be replaced with launcher script: $static_core_elf" >&2
    exit 1
  fi
else
  echo "error: expected wrapped launcher to remain at static provider path: $static_core_elf" >&2
  exit 1
fi
if [[ ! -f "$non_elf" ]]; then
  echo "error: expected non-ELF artifact to remain: $non_elf" >&2
  exit 1
fi

echo "ok: linux_bundle_prune_glibc enforces arch+libc ELF contract"
