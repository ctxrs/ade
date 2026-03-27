#!/usr/bin/env bash
set -euo pipefail

input_archive=""
output_archive=""

usage() {
  cat <<'EOF'
usage: curate_avf_container_stack.sh --input ARCHIVE --output ARCHIVE

Repack a nerdctl-full Linux guest archive down to the binaries/plugins required by
the current AVF guest contract.
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

curated_entries() {
  cat <<'EOF'
bin/buildctl
bin/buildkitd
bin/containerd
bin/containerd-shim-runc-v2
bin/ctr
bin/nerdctl
bin/runc
libexec/cni/bridge
libexec/cni/firewall
libexec/cni/host-local
libexec/cni/loopback
libexec/cni/portmap
libexec/cni/tuning
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --input)
      input_archive="${2:-}"
      shift 2
      ;;
    --output)
      output_archive="${2:-}"
      shift 2
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

[[ -n "$input_archive" ]] || die "--input is required"
[[ -n "$output_archive" ]] || die "--output is required"
[[ -f "$input_archive" ]] || die "input archive does not exist: $input_archive"

need_cmd tar

tmp_root="$(mktemp -d "${TMPDIR:-/tmp}/ctx-avf-container-stack-curate.XXXXXX")"
cleanup() {
  chmod -R u+rwX "$tmp_root" 2>/dev/null || true
  rm -rf "$tmp_root"
}
trap cleanup EXIT

extract_root="$tmp_root/extracted"
curated_root="$tmp_root/curated"
mkdir -p "$extract_root" "$curated_root"

tar -xzf "$input_archive" -C "$extract_root"

while IFS= read -r relative_path; do
  [[ -n "$relative_path" ]] || continue
  source_path="$extract_root/$relative_path"
  [[ -f "$source_path" ]] || die "required container-stack entry is missing: $relative_path"
  destination_path="$curated_root/$relative_path"
  mkdir -p "$(dirname "$destination_path")"
  cp -p "$source_path" "$destination_path"
done < <(curated_entries)

mkdir -p "$(dirname "$output_archive")"
rm -f "$output_archive"
tar -czf "$output_archive" -C "$curated_root" .

