const DEFAULT_FUNCTIONS_BASE = "https://api.ctx.rs/functions/v1";
const DEFAULT_CHANNEL = "stable";

export function renderControlPlaneInstallScript({
  functionsBase = DEFAULT_FUNCTIONS_BASE,
  channel = DEFAULT_CHANNEL,
} = {}) {
  const normalizedBase = String(functionsBase).replace(/\/+$/, "");
  const normalizedChannel = String(channel).trim() || DEFAULT_CHANNEL;
  return `#!/bin/sh
set -eu

log() {
  printf '%s\\n' "$*" >&2
}

fail() {
  log "error: $*"
  exit 1
}

need_cmd() {
  command -v "$1" >/dev/null 2>&1 || fail "missing required command: $1"
}

need_cmd curl
need_cmd mktemp
need_cmd uname
need_cmd mkdir
need_cmd mv
need_cmd cp
need_cmd chmod
need_cmd tar
need_cmd find
need_cmd head
need_cmd awk

functions_base="\${CTX_CONTROL_PLANE_FUNCTIONS_BASE:-${normalizedBase}}"
channel="\${CTX_CONTROL_PLANE_CHANNEL:-${normalizedChannel}}"
manifest_url="\${functions_base%/}/releases/$channel/control-plane/latest.json"
os="$(uname -s)"
arch="$(uname -m)"

tmp_dir="$(mktemp -d "\${TMPDIR:-/tmp}/ctx-control-plane-install.XXXXXX")"
manifest_json="$tmp_dir/latest.json"
archive_path="$tmp_dir/ctx-control-plane.tar.gz"
extract_dir="$tmp_dir/extract"
cleanup_stage_path=""
cleanup_backup_path=""

cleanup() {
  if [ -n "$cleanup_stage_path" ] && [ -e "$cleanup_stage_path" ]; then
    rm -rf "$cleanup_stage_path"
  fi
  if [ -n "$cleanup_backup_path" ] && [ -e "$cleanup_backup_path" ]; then
    rm -rf "$cleanup_backup_path"
  fi
  rm -rf "$tmp_dir"
}
trap cleanup EXIT INT TERM

unique_target_sibling_path() {
  target_path="$1"
  label="$2"
  candidate="$target_path.$label.$$"
  index=0
  while [ -e "$candidate" ]; do
    index=$((index + 1))
    candidate="$target_path.$label.$$.\${index}"
  done
  printf "%s\\n" "$candidate"
}

stage_path_for_target() {
  unique_target_sibling_path "$1" "ctx-stage"
}

backup_path_for_target() {
  unique_target_sibling_path "$1" "ctx-backup"
}

promote_staged_path() {
  staged_path="$1"
  target_path="$2"
  target_label="$3"
  backup_path=""

  cleanup_stage_path="$staged_path"
  cleanup_backup_path=""

  if [ -e "$target_path" ]; then
    backup_path="$(backup_path_for_target "$target_path")"
    mv "$target_path" "$backup_path" || fail "failed to move existing $target_label aside"
    cleanup_backup_path="$backup_path"
  fi

  if mv "$staged_path" "$target_path"; then
    cleanup_stage_path=""
    if [ -n "$backup_path" ] && [ -e "$backup_path" ]; then
      rm -rf "$backup_path" || fail "failed to remove previous $target_label backup"
    fi
    cleanup_backup_path=""
    return 0
  fi

  if [ -n "$backup_path" ] && [ -e "$backup_path" ]; then
    mv "$backup_path" "$target_path" || fail "failed to restore previous $target_label after upgrade failure"
    cleanup_backup_path=""
  fi

  fail "failed to install $target_label"
}

extract_manifest_field() {
  key="$1"
  if command -v python3 >/dev/null 2>&1; then
    PYTHONHOME= PYTHONPATH= python3 - "$manifest_json" "$key" <<'PY'
import json
import sys

manifest_path = sys.argv[1]
key_path = sys.argv[2]
parts = key_path.split(".")
with open(manifest_path, "r", encoding="utf-8") as f:
    data = json.load(f)
for part in parts:
    if isinstance(data, dict) and part in data:
        data = data[part]
    else:
        print("")
        raise SystemExit(0)
if isinstance(data, (str, int, float, bool)):
    print(data)
else:
    print("")
PY
    return 0
  fi
  if command -v plutil >/dev/null 2>&1; then
    plutil -extract "$key" raw -o - "$manifest_json" 2>/dev/null || true
    return 0
  fi
  fail "missing required command: python3 (or plutil)"
}

sha256_of_file() {
  file_path="$1"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$file_path" | awk '{print $1}'
    return 0
  fi
  if command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$file_path" | awk '{print $1}'
    return 0
  fi
  fail "missing required command: sha256sum (or shasum)"
}

verify_artifact_sha() {
  artifact_path="$1"
  expected_sha="$2"
  if [ -z "$expected_sha" ]; then
    fail "manifest missing sha256 for selected artifact"
  fi
  actual_sha="$(sha256_of_file "$artifact_path")"
  if [ "$actual_sha" != "$expected_sha" ]; then
    fail "sha256 mismatch (expected $expected_sha, got $actual_sha)"
  fi
  log "Verified artifact sha256"
}

resolve_download_url() {
  url_path="$1"
  case "$url_path" in
    http://*|https://*) printf "%s\\n" "$url_path" ;;
    /*) printf "%s\\n" "\${functions_base%/}$url_path" ;;
    *) printf "%s\\n" "\${functions_base%/}/$url_path" ;;
  esac
}

select_platform() {
  case "$os:$arch" in
    Darwin:arm64) printf '%s\\n' "macos-arm64" ;;
    Darwin:x86_64) printf '%s\\n' "macos-x64" ;;
    Linux:x86_64|Linux:amd64) printf '%s\\n' "linux-x64" ;;
    Linux:aarch64|Linux:arm64) printf '%s\\n' "linux-arm64" ;;
    *) fail "unsupported platform: $os $arch" ;;
  esac
}

find_ctx_binary() {
  find "$extract_dir" -type f -name ctx | head -n 1
}

platform="$(select_platform)"

log "Resolving ctx control plane release manifest from $manifest_url"
curl -fsSL "$manifest_url" -o "$manifest_json"

url_path="$(extract_manifest_field "platforms.$platform.cli.url_path")"
if [ -z "$url_path" ]; then
  url_path="$(extract_manifest_field "platforms.$platform.archive.url_path")"
fi
[ -n "$url_path" ] || fail "manifest does not contain a ctx control plane CLI archive for $platform"

expected_sha="$(extract_manifest_field "platforms.$platform.cli.sha256")"
if [ -z "$expected_sha" ]; then
  expected_sha="$(extract_manifest_field "platforms.$platform.archive.sha256")"
fi

download_url="$(resolve_download_url "$url_path")"
log "Downloading $download_url"
curl -fL --retry 3 --retry-delay 2 "$download_url" -o "$archive_path"
verify_artifact_sha "$archive_path" "$expected_sha"

mkdir -p "$extract_dir"
tar -xzf "$archive_path" -C "$extract_dir"
ctx_binary="$(find_ctx_binary)"
[ -n "$ctx_binary" ] || fail "downloaded archive does not contain an executable ctx binary"

install_root="\${CTX_CONTROL_PLANE_INSTALL_DIR:-$HOME/.local/share/ctx-control-plane}"
bin_dir="\${CTX_CONTROL_PLANE_BIN_DIR:-$HOME/.local/bin}"
mkdir -p "$install_root" "$bin_dir"

target_binary="$install_root/ctx"
staged_binary="$(stage_path_for_target "$target_binary")"
cp "$ctx_binary" "$staged_binary"
chmod 0755 "$staged_binary"
promote_staged_path "$staged_binary" "$target_binary" "ctx control plane binary"

target_link="$bin_dir/ctx"
staged_link="$(stage_path_for_target "$target_link")"
cat >"$staged_link" <<EOF
#!/bin/sh
exec "$target_binary" "\\$@"
EOF
chmod 0755 "$staged_link"
promote_staged_path "$staged_link" "$target_link" "ctx launcher"

log "Installed ctx control plane at $target_binary"
log "Installed ctx launcher at $target_link"
if ! command -v ctx >/dev/null 2>&1; then
  log "Add $bin_dir to PATH to run ctx from any shell."
fi
log "Done."
`;
}
