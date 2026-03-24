#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
runtime_dir=""
guest_agent_path=""
egress_proxy_path=""
arch=""
release_dir="https://cloud-images.ubuntu.com/releases/noble/release"
kernel_cmdline="console=hvc0 root=LABEL=cloudimg-rootfs rootwait rw"
force=0
dry_run=0

usage() {
  cat <<'EOF'
usage: prepare_avf_linux_guest_runtime.sh --output-dir DIR [options]

Prepare a local AVF Linux guest runtime directory from official Ubuntu artifacts.

Required:
  --output-dir DIR       Destination runtime directory

Options:
  --arch ARCH            Guest arch: arm64 or x86_64 (default: host arch)
  --guest-agent PATH     Guest-agent binary to stage (default: auto-discover)
  --egress-proxy PATH    ctx-egress-proxy Linux binary to stage (default: auto-discover)
  --release-dir URL      Ubuntu release directory (default: noble release feed)
  --force                Replace an existing output directory
  --dry-run              Print the resolved inputs and exit without downloading
  -h, --help             Show this help
EOF
}

die() {
  echo "error: $*" >&2
  exit 1
}

need_cmd() {
  local cmd="$1"
  command -v "$cmd" >/dev/null 2>&1 || die "missing required command: $cmd"
}

resolve_qemu_img() {
  if command -v qemu-img >/dev/null 2>&1; then
    command -v qemu-img
    return 0
  fi
  local brew_prefix
  if command -v brew >/dev/null 2>&1; then
    brew_prefix="$(brew --prefix qemu 2>/dev/null || true)"
    if [[ -n "$brew_prefix" && -x "$brew_prefix/bin/qemu-img" ]]; then
      printf '%s' "$brew_prefix/bin/qemu-img"
      return 0
    fi
  fi
  return 1
}

normalize_arch() {
  case "$1" in
    arm64|aarch64) printf '%s' "arm64" ;;
    x86_64|amd64) printf '%s' "x86_64" ;;
    *) die "unsupported arch: $1 (expected arm64 or x86_64)" ;;
  esac
}

ubuntu_image_arch() {
  case "$1" in
    arm64) printf '%s' "arm64" ;;
    x86_64) printf '%s' "amd64" ;;
    *) die "unsupported arch: $1" ;;
  esac
}

guest_agent_target() {
  case "$1" in
    arm64) printf '%s' "aarch64-unknown-linux-gnu" ;;
    x86_64) printf '%s' "x86_64-unknown-linux-gnu" ;;
    *) die "unsupported arch: $1" ;;
  esac
}

default_host_arch() {
  case "$(uname -m)" in
    arm64|aarch64) printf '%s' "arm64" ;;
    x86_64|amd64) printf '%s' "x86_64" ;;
    *) die "unsupported host arch: $(uname -m)" ;;
  esac
}

discover_guest_agent() {
  local target="$1"
  local candidates=(
    "${repo_root}/target/${target}/release/ctx-avf-linux-guest-agent"
    "${repo_root}/target/${target}/debug/ctx-avf-linux-guest-agent"
  )
  for candidate in "${candidates[@]}"; do
    if [[ -f "$candidate" ]]; then
      printf '%s' "$candidate"
      return 0
    fi
  done
  return 1
}

discover_egress_proxy() {
  local target="$1"
  local candidates=(
    "${repo_root}/target/${target}/release/ctx-egress-proxy"
    "${repo_root}/target/${target}/debug/ctx-egress-proxy"
  )
  for candidate in "${candidates[@]}"; do
    if [[ -f "$candidate" ]]; then
      printf '%s' "$candidate"
      return 0
    fi
  done
  return 1
}

download_file() {
  local url="$1"
  local dest="$2"
  curl -fL --retry 3 --retry-delay 1 -o "$dest" "$url"
}

sha256_file() {
  local file="$1"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$file" | awk '{print $1}'
  else
    shasum -a 256 "$file" | awk '{print $1}'
  fi
}

lookup_expected_sha256() {
  local sums_file="$1"
  local filename="$2"
  awk -v file="$filename" '
    {
      name = $2
      sub(/^\*/, "", name)
      if (name == file) {
        print $1
        exit
      }
    }
  ' "$sums_file"
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --output-dir)
      runtime_dir="${2:-}"
      shift 2
      ;;
    --guest-agent)
      guest_agent_path="${2:-}"
      shift 2
      ;;
    --egress-proxy)
      egress_proxy_path="${2:-}"
      shift 2
      ;;
    --arch)
      arch="${2:-}"
      shift 2
      ;;
    --release-dir)
      release_dir="${2:-}"
      shift 2
      ;;
    --force)
      force=1
      shift
      ;;
    --dry-run)
      dry_run=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      die "unknown argument: $1"
      ;;
  esac
done

[[ -n "$runtime_dir" ]] || die "--output-dir is required"

if [[ -z "$arch" ]]; then
  arch="$(default_host_arch)"
else
  arch="$(normalize_arch "$arch")"
fi

ubuntu_arch="$(ubuntu_image_arch "$arch")"
target="$(guest_agent_target "$arch")"

if [[ -z "$guest_agent_path" ]]; then
  if ! guest_agent_path="$(discover_guest_agent "$target")"; then
    die "could not find ctx-avf-linux-guest-agent for target ${target}; build it first or pass --guest-agent"
  fi
fi

[[ -f "$guest_agent_path" ]] || die "guest-agent binary does not exist: $guest_agent_path"

if [[ -z "$egress_proxy_path" ]]; then
  if ! egress_proxy_path="$(discover_egress_proxy "$target")"; then
    die "could not find ctx-egress-proxy for target ${target}; build it first or pass --egress-proxy"
  fi
fi

[[ -f "$egress_proxy_path" ]] || die "egress proxy binary does not exist: $egress_proxy_path"

runtime_dir="$(cd "$(dirname "$runtime_dir")" && pwd)/$(basename "$runtime_dir")"
if [[ -e "$runtime_dir" ]]; then
  if [[ -d "$runtime_dir" ]] && [[ -n "$(find "$runtime_dir" -mindepth 1 -maxdepth 1 -print -quit 2>/dev/null || true)" ]]; then
    [[ "$force" -eq 1 ]] || die "output directory exists and is not empty: $runtime_dir (use --force to replace it)"
    rm -rf "$runtime_dir"
  elif [[ -d "$runtime_dir" ]]; then
    rm -rf "$runtime_dir"
  else
    [[ "$force" -eq 1 ]] || die "output path exists and is not a directory: $runtime_dir"
    rm -f "$runtime_dir"
  fi
fi

rootfs_name="ubuntu-24.04-server-cloudimg-${ubuntu_arch}.img"
kernel_name="ubuntu-24.04-server-cloudimg-${ubuntu_arch}-vmlinuz-generic"
initrd_name="ubuntu-24.04-server-cloudimg-${ubuntu_arch}-initrd-generic"

rootfs_url="${release_dir}/${rootfs_name}"
kernel_url="${release_dir}/unpacked/${kernel_name}"
initrd_url="${release_dir}/unpacked/${initrd_name}"
rootfs_sha256_url="${release_dir}/SHA256SUMS"
unpacked_sha256_url="${release_dir}/unpacked/SHA256SUMS"

if [[ "$dry_run" -eq 1 ]]; then
  cat <<EOF
runtime_dir=$runtime_dir
arch=$arch
ubuntu_arch=$ubuntu_arch
guest_agent=$guest_agent_path
rootfs_path=$runtime_dir/rootfs.raw
kernel_path=$runtime_dir/helpers/kernel
initrd_path=$runtime_dir/helpers/initrd
kernel_cmdline_path=$runtime_dir/helpers/kernel-cmdline
kernel_cmdline=$kernel_cmdline
guest_agent_runtime_path=$runtime_dir/helpers/guest-agent
egress_proxy_runtime_path=$runtime_dir/helpers/egress-proxy
rootfs_url=$rootfs_url
kernel_url=$kernel_url
initrd_url=$initrd_url
rootfs_sha256_url=$rootfs_sha256_url
unpacked_sha256_url=$unpacked_sha256_url
EOF
  exit 0
fi

need_cmd curl
if ! qemu_img_bin="$(resolve_qemu_img)"; then
  die "missing qemu-img; install qemu via Homebrew"
fi

tmp_root="$(mktemp -d "${TMPDIR:-/tmp}/ctx-avf-linux-guest-runtime.XXXXXX")"
cleanup() {
  chmod -R u+rwX "$tmp_root" 2>/dev/null || true
  rm -rf "$tmp_root"
}
trap cleanup EXIT

rootfs_qcow="$tmp_root/$rootfs_name"
kernel_path="$tmp_root/$kernel_name"
initrd_path="$tmp_root/$initrd_name"
rootfs_sha256_sums="$tmp_root/SHA256SUMS"
unpacked_sha256_sums="$tmp_root/unpacked.SHA256SUMS"

mkdir -p "$runtime_dir/helpers"

download_file "$rootfs_sha256_url" "$rootfs_sha256_sums"
download_file "$unpacked_sha256_url" "$unpacked_sha256_sums"
download_file "$rootfs_url" "$rootfs_qcow"
download_file "$kernel_url" "$kernel_path"
download_file "$initrd_url" "$initrd_path"

for item in \
  "$rootfs_sha256_sums:$rootfs_name:$rootfs_qcow" \
  "$unpacked_sha256_sums:$kernel_name:$kernel_path" \
  "$unpacked_sha256_sums:$initrd_name:$initrd_path"
do
  item_sums="${item%%:*}"
  item_rest="${item#*:}"
  item_name="${item_rest%%:*}"
  item_path="${item_rest#*:}"
  expected="$(lookup_expected_sha256 "$item_sums" "$item_name")"
  [[ -n "$expected" ]] || die "missing SHA256SUMS entry for $item_name"
  actual="$(sha256_file "$item_path")"
  [[ "$actual" == "$expected" ]] || die "sha256 mismatch for $item_name"
done

rootfs_image="$tmp_root/rootfs.raw"
"$qemu_img_bin" convert -f qcow2 -O raw "$rootfs_qcow" "$rootfs_image"

rootfs_raw_sha256="$(sha256_file "$rootfs_image")"
kernel_sha256="$(sha256_file "$kernel_path")"
initrd_sha256="$(sha256_file "$initrd_path")"
runtime_version="ubuntu-noble-${ubuntu_arch}-${rootfs_raw_sha256:0:12}"

install -m 0644 "$kernel_path" "$runtime_dir/helpers/kernel"
install -m 0644 "$initrd_path" "$runtime_dir/helpers/initrd"
printf '%s\n' "$kernel_cmdline" > "$runtime_dir/helpers/kernel-cmdline"
install -m 0755 "$guest_agent_path" "$runtime_dir/helpers/guest-agent"
install -m 0755 "$egress_proxy_path" "$runtime_dir/helpers/egress-proxy"
cp -c "$rootfs_image" "$runtime_dir/rootfs.raw" 2>/dev/null || cp -p "$rootfs_image" "$runtime_dir/rootfs.raw"

cat > "$runtime_dir/version.txt" <<EOF
version=$runtime_version
ubuntu-release=noble
ubuntu-arch=$ubuntu_arch
guest-agent-target=$target
egress-proxy-target=$target
source-rootfs=$rootfs_url
source-kernel=$kernel_url
source-initrd=$initrd_url
rootfs-sha256=$rootfs_raw_sha256
kernel-sha256=$kernel_sha256
initrd-sha256=$initrd_sha256
kernel-cmdline=$kernel_cmdline
rootfs-format=raw
guest-agent-preinstalled=false
EOF

cat <<EOF
prepared AVF Linux guest runtime at $runtime_dir
  rootfs: $runtime_dir/rootfs.raw
  kernel: $runtime_dir/helpers/kernel
  initrd: $runtime_dir/helpers/initrd
  kernel-cmdline: $runtime_dir/helpers/kernel-cmdline
  guest-agent: $runtime_dir/helpers/guest-agent
  egress-proxy: $runtime_dir/helpers/egress-proxy
EOF
