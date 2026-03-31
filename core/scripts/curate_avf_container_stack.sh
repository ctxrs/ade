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
need_cmd python3

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

find "$curated_root" -exec touch -h -t 197001010000 {} +
archive_entries="$tmp_root/archive-entries.txt"
(cd "$curated_root" && LC_ALL=C find . -type f -print | LC_ALL=C sort > "$archive_entries")

mkdir -p "$(dirname "$output_archive")"
rm -f "$output_archive"
python3 - "$curated_root" "$archive_entries" "$output_archive" <<'PY'
import gzip
import os
import sys
import tarfile

curated_root, entries_path, output_archive = sys.argv[1:4]
with open(entries_path, "r", encoding="utf-8") as handle:
    entries = [line.strip() for line in handle if line.strip()]

with open(output_archive, "wb") as raw_handle:
    with gzip.GzipFile(filename="", mode="wb", fileobj=raw_handle, mtime=0) as gz_handle:
        with tarfile.open(mode="w", fileobj=gz_handle, format=tarfile.GNU_FORMAT) as archive:
            for entry in entries:
                relative_entry = entry[2:] if entry.startswith("./") else entry
                source_path = os.path.join(curated_root, relative_entry)
                info = archive.gettarinfo(source_path, arcname=entry)
                info.uid = 0
                info.gid = 0
                info.uname = ""
                info.gname = ""
                info.mtime = 0
                with open(source_path, "rb") as source_handle:
                    archive.addfile(info, source_handle)
PY
