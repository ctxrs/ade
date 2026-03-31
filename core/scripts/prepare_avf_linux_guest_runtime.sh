#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
runtime_dir=""
guest_agent_path=""
egress_proxy_path=""
guest_agent_override=0
egress_proxy_override=0
container_stack_path=""
target_root=""
helper_build_tmp_root=""
arch=""
release_dir="https://cloud-images.ubuntu.com/releases/noble/release"
kernel_cmdline="console=hvc0 root=LABEL=cloudimg-rootfs rootwait rw"
container_stack_version="${NERDCTL_VERSION:-v2.2.1}"
container_stack_mode="curated-nerdctl-subset"
force=0
dry_run=0
auto_built_guest_helpers=0

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
  --container-stack PATH Prebuilt Linux guest container-stack tarball to stage
                         (default: download pinned nerdctl-full release)
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

cleanup() {
  if [[ -n "${helper_build_tmp_root}" ]]; then
    chmod -R u+rwX "${helper_build_tmp_root}" 2>/dev/null || true
    rm -rf "${helper_build_tmp_root}"
  fi
  if [[ -n "${tmp_root:-}" ]]; then
    chmod -R u+rwX "${tmp_root}" 2>/dev/null || true
    rm -rf "${tmp_root}"
  fi
}
trap cleanup EXIT

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

container_stack_asset_arch() {
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
    "${target_root}/${target}/release/ctx-avf-linux-guest-agent"
    "${target_root}/${target}/debug/ctx-avf-linux-guest-agent"
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
    "${target_root}/${target}/release/ctx-egress-proxy"
    "${target_root}/${target}/debug/ctx-egress-proxy"
  )
  for candidate in "${candidates[@]}"; do
    if [[ -f "$candidate" ]]; then
      printf '%s' "$candidate"
      return 0
    fi
  done
  return 1
}

build_guest_helpers() {
  local target="$1"
  local build_script="${CTX_AVF_BUILD_AVF_LINUX_GUEST_HELPERS_SCRIPT:-${repo_root}/scripts/build_avf_linux_guest_agent.sh}"
  [[ -f "$build_script" ]] || die "guest helper build script does not exist: $build_script"
  CTX_AVF_GUEST_HELPER_TARGETS="$target" \
  CTX_AVF_GUEST_HELPER_TARGET_ROOT="$target_root" \
    bash "$build_script" --release
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

compute_runtime_version_suffix() {
  local ubuntu_arch="$1"
  local rootfs_sha="$2"
  local kernel_sha="$3"
  local initrd_sha="$4"
  local guest_agent_sha="$5"
  local egress_proxy_sha="$6"
  local container_stack_sha="$7"
  local kernel_cmdline_value="$8"
  local combined
  combined="$(printf '%s\n' \
    "ubuntu_arch=$ubuntu_arch" \
    "rootfs=$rootfs_sha" \
    "kernel=$kernel_sha" \
    "initrd=$initrd_sha" \
    "guest_agent=$guest_agent_sha" \
    "egress_proxy=$egress_proxy_sha" \
    "container_stack=$container_stack_sha" \
    "kernel_cmdline=$kernel_cmdline_value")"
  if command -v sha256sum >/dev/null 2>&1; then
    printf '%s' "$(printf '%s' "$combined" | sha256sum | awk '{print substr(tolower($1), 1, 12)}')"
  else
    printf '%s' "$(printf '%s' "$combined" | shasum -a 256 | awk '{print substr(tolower($1), 1, 12)}')"
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

container_stack_filename() {
  local version="$1"
  local asset_arch="$2"
  printf 'nerdctl-full-%s-linux-%s.tar.gz' "${version#v}" "$asset_arch"
}

curated_container_stack_inventory() {
  printf '%s' 'bin/buildctl,bin/buildkitd,bin/containerd,bin/containerd-shim-runc-v2,bin/ctr,bin/nerdctl,bin/runc,libexec/cni/bridge,libexec/cni/firewall,libexec/cni/host-local,libexec/cni/loopback,libexec/cni/portmap,libexec/cni/tuning'
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --output-dir)
      runtime_dir="${2:-}"
      shift 2
      ;;
    --guest-agent)
      guest_agent_path="${2:-}"
      guest_agent_override=1
      shift 2
      ;;
    --egress-proxy)
      egress_proxy_path="${2:-}"
      egress_proxy_override=1
      shift 2
      ;;
    --arch)
      arch="${2:-}"
      shift 2
      ;;
    --container-stack)
      container_stack_path="${2:-}"
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
container_stack_arch="$(container_stack_asset_arch "$arch")"
target="$(guest_agent_target "$arch")"
needs_guest_helper_build=0
if [[ "$guest_agent_override" -eq 0 || "$egress_proxy_override" -eq 0 ]]; then
  needs_guest_helper_build=1
fi
target_root="${CTX_AVF_GUEST_HELPER_TARGET_ROOT:-}"
if [[ -z "$target_root" ]]; then
  if [[ "$needs_guest_helper_build" -eq 1 ]]; then
    helper_build_tmp_root="$(mktemp -d "${TMPDIR:-/tmp}/ctx-avf-guest-helper-target.XXXXXX")"
    target_root="$helper_build_tmp_root"
  else
    target_root="${CARGO_TARGET_DIR:-${repo_root}/target}"
  fi
fi

if [[ -z "$guest_agent_path" ]]; then
  guest_agent_path="$(discover_guest_agent "$target" || true)"
fi

if [[ -z "$egress_proxy_path" ]]; then
  egress_proxy_path="$(discover_egress_proxy "$target" || true)"
fi

if [[ "$guest_agent_override" -eq 0 || "$egress_proxy_override" -eq 0 ]]; then
  build_guest_helpers "$target"
  auto_built_guest_helpers=1
fi

if [[ -z "$guest_agent_path" ]]; then
  guest_agent_path="$(discover_guest_agent "$target" || true)"
fi
[[ -n "$guest_agent_path" ]] || die "could not find ctx-avf-linux-guest-agent for target ${target} under ${target_root} after building; pass --guest-agent to override"
[[ -f "$guest_agent_path" ]] || die "guest-agent binary does not exist: $guest_agent_path"

if [[ -z "$egress_proxy_path" ]]; then
  egress_proxy_path="$(discover_egress_proxy "$target" || true)"
fi
[[ -n "$egress_proxy_path" ]] || die "could not find ctx-egress-proxy for target ${target} under ${target_root} after building; pass --egress-proxy to override"
[[ -f "$egress_proxy_path" ]] || die "egress proxy binary does not exist: $egress_proxy_path"

container_stack_name="$(container_stack_filename "$container_stack_version" "$container_stack_arch")"
container_stack_url="https://github.com/containerd/nerdctl/releases/download/${container_stack_version}/${container_stack_name}"
container_stack_sha256_url="https://github.com/containerd/nerdctl/releases/download/${container_stack_version}/SHA256SUMS"
container_stack_curator="${repo_root}/scripts/curate_avf_container_stack.sh"

if [[ -n "$container_stack_path" ]]; then
  [[ -f "$container_stack_path" ]] || die "container stack archive does not exist: $container_stack_path"
fi
[[ -f "$container_stack_curator" ]] || die "container stack curator script does not exist: $container_stack_curator"

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
target_root=$target_root
guest_agent=$guest_agent_path
egress_proxy=$egress_proxy_path
auto_built_guest_helpers=$auto_built_guest_helpers
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
container_stack_version=$container_stack_version
container_stack_mode=$container_stack_mode
container_stack_inventory=$(curated_container_stack_inventory)
container_stack_path=$runtime_dir/helpers/container-stack.tar.gz
container_stack_url=$container_stack_url
container_stack_sha256_url=$container_stack_sha256_url
rootfs_sha256_url=$rootfs_sha256_url
unpacked_sha256_url=$unpacked_sha256_url
runtime_version_strategy=artifact-digests
runtime_version_inputs=rootfs,kernel,initrd,guest-agent,egress-proxy,container-stack,kernel-cmdline
EOF
  exit 0
fi

need_cmd curl
if ! qemu_img_bin="$(resolve_qemu_img)"; then
  die "missing qemu-img; install qemu via Homebrew"
fi

tmp_root="$(mktemp -d "${TMPDIR:-/tmp}/ctx-avf-linux-guest-runtime.XXXXXX")"

rootfs_qcow="$tmp_root/$rootfs_name"
kernel_path="$tmp_root/$kernel_name"
initrd_path="$tmp_root/$initrd_name"
rootfs_sha256_sums="$tmp_root/SHA256SUMS"
unpacked_sha256_sums="$tmp_root/unpacked.SHA256SUMS"
container_stack_sha256_sums="$tmp_root/nerdctl-full.SHA256SUMS"
container_stack_archive_raw="$tmp_root/$container_stack_name"
container_stack_archive_curated="$tmp_root/container-stack.curated.tar.gz"

mkdir -p "$runtime_dir/helpers"

download_file "$rootfs_sha256_url" "$rootfs_sha256_sums"
download_file "$unpacked_sha256_url" "$unpacked_sha256_sums"
download_file "$container_stack_sha256_url" "$container_stack_sha256_sums"
download_file "$rootfs_url" "$rootfs_qcow"
download_file "$kernel_url" "$kernel_path"
download_file "$initrd_url" "$initrd_path"
if [[ -n "$container_stack_path" ]]; then
  cp -p "$container_stack_path" "$container_stack_archive_raw"
else
  download_file "$container_stack_url" "$container_stack_archive_raw"
fi

for item in \
  "$rootfs_sha256_sums:$rootfs_name:$rootfs_qcow" \
  "$unpacked_sha256_sums:$kernel_name:$kernel_path" \
  "$unpacked_sha256_sums:$initrd_name:$initrd_path" \
  "$container_stack_sha256_sums:$container_stack_name:$container_stack_archive_raw"
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

bash "$container_stack_curator" \
  --input "$container_stack_archive_raw" \
  --output "$container_stack_archive_curated"

rootfs_image="$tmp_root/rootfs.raw"
"$qemu_img_bin" convert -f qcow2 -O raw "$rootfs_qcow" "$rootfs_image"

rootfs_raw_sha256="$(sha256_file "$rootfs_image")"
kernel_sha256="$(sha256_file "$kernel_path")"
initrd_sha256="$(sha256_file "$initrd_path")"
guest_agent_sha256="$(sha256_file "$guest_agent_path")"
egress_proxy_sha256="$(sha256_file "$egress_proxy_path")"
container_stack_sha256="$(sha256_file "$container_stack_archive_curated")"
runtime_version_suffix="$(
  compute_runtime_version_suffix \
    "$ubuntu_arch" \
    "$rootfs_raw_sha256" \
    "$kernel_sha256" \
    "$initrd_sha256" \
    "$guest_agent_sha256" \
    "$egress_proxy_sha256" \
    "$container_stack_sha256" \
    "$kernel_cmdline"
)"
runtime_version="ubuntu-noble-${ubuntu_arch}-${runtime_version_suffix}"

install -m 0644 "$kernel_path" "$runtime_dir/helpers/kernel"
install -m 0644 "$initrd_path" "$runtime_dir/helpers/initrd"
printf '%s\n' "$kernel_cmdline" > "$runtime_dir/helpers/kernel-cmdline"
install -m 0755 "$guest_agent_path" "$runtime_dir/helpers/guest-agent"
install -m 0755 "$egress_proxy_path" "$runtime_dir/helpers/egress-proxy"
install -m 0644 "$container_stack_archive_curated" "$runtime_dir/helpers/container-stack.tar.gz"
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
guest-agent-sha256=$guest_agent_sha256
egress-proxy-sha256=$egress_proxy_sha256
kernel-cmdline=$kernel_cmdline
rootfs-format=raw
guest-agent-preinstalled=false
container-stack-version=$container_stack_version
container-stack-archive=$container_stack_name
container-stack-source=$container_stack_url
container-stack-sha256=$container_stack_sha256
container-stack-mode=$container_stack_mode
container-stack-inventory=$(curated_container_stack_inventory)
EOF

cat <<EOF
prepared AVF Linux guest runtime at $runtime_dir
  rootfs: $runtime_dir/rootfs.raw
  kernel: $runtime_dir/helpers/kernel
  initrd: $runtime_dir/helpers/initrd
  kernel-cmdline: $runtime_dir/helpers/kernel-cmdline
  guest-agent: $runtime_dir/helpers/guest-agent
  egress-proxy: $runtime_dir/helpers/egress-proxy
  container-stack: $runtime_dir/helpers/container-stack.tar.gz
EOF
