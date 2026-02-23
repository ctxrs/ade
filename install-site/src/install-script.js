const DEFAULT_FUNCTIONS_BASE = "https://api.ctx.rs/functions/v1";
const DEFAULT_CHANNEL = "stable";

export function renderInstallScript({
  functionsBase = DEFAULT_FUNCTIONS_BASE,
  channel = DEFAULT_CHANNEL,
} = {}) {
  const normalizedBase = String(functionsBase).replace(/\/+$/, "");
  const normalizedChannel = String(channel);
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
need_cmd find
need_cmd awk
need_cmd uname

functions_base="\${CTX_FUNCTIONS_BASE:-${normalizedBase}}"
channel="\${CTX_CHANNEL:-${normalizedChannel}}"
manifest_url="\${functions_base%/}/releases/$channel/latest.json"
os="$(uname -s)"
arch="$(uname -m)"

tmp_dir="$(mktemp -d "\${TMPDIR:-/tmp}/ctx-install.XXXXXX")"
mount_dir="$tmp_dir/mount"
manifest_json="$tmp_dir/latest.json"
mounted=0

cleanup() {
  if [ "$mounted" = "1" ]; then
    if command -v hdiutil >/dev/null 2>&1; then
      hdiutil detach "$mount_dir" -quiet >/dev/null 2>&1 || true
    fi
  fi
  rm -rf "$tmp_dir"
}
trap cleanup EXIT INT TERM

extract_manifest_field_macos() {
  key="$1"
  plutil -extract "$key" raw -o - "$manifest_json" 2>/dev/null || true
}

extract_manifest_field_linux() {
  key="$1"
  python3 - "$manifest_json" "$key" <<'PY'
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
}

resolve_download_url() {
  url_path="$1"
  case "$url_path" in
    http://*|https://*) printf "%s\\n" "$url_path" ;;
    /*) printf "%s\\n" "\${functions_base%/}$url_path" ;;
    *) printf "%s\\n" "\${functions_base%/}/$url_path" ;;
  esac
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
    log "Warning: manifest missing sha256; skipping checksum verification"
    return 0
  fi
  actual_sha="$(sha256_of_file "$artifact_path")"
  if [ "$actual_sha" != "$expected_sha" ]; then
    fail "sha256 mismatch (expected $expected_sha, got $actual_sha)"
  fi
  log "Verified artifact sha256"
}

install_macos() {
  local_arch="$1"
  need_cmd hdiutil
  need_cmd plutil
  need_cmd ditto
  need_cmd open

  case "$local_arch" in
    arm64) platform="macos-arm64" ;;
    x86_64) platform="macos-x64" ;;
    *) fail "unsupported macOS architecture: $local_arch" ;;
  esac

  url_path="$(extract_manifest_field_macos "platforms.$platform.desktop.url_path")"
  if [ -z "$url_path" ]; then
    url_path="$(extract_manifest_field_macos "platforms.$platform.dmg.url_path")"
  fi
  [ -n "$url_path" ] || fail "manifest does not contain a desktop dmg for $platform"

  expected_sha="$(extract_manifest_field_macos "platforms.$platform.desktop.sha256")"
  if [ -z "$expected_sha" ]; then
    expected_sha="$(extract_manifest_field_macos "platforms.$platform.dmg.sha256")"
  fi

  artifact_path="$tmp_dir/ctx-installer.dmg"
  download_url="$(resolve_download_url "$url_path")"
  log "Downloading $download_url"
  curl -fL --retry 3 --retry-delay 2 "$download_url" -o "$artifact_path"
  verify_artifact_sha "$artifact_path" "$expected_sha"

  mkdir -p "$mount_dir"
  log "Mounting DMG"
  hdiutil attach "$artifact_path" -nobrowse -readonly -mountpoint "$mount_dir" >/dev/null
  mounted=1

  app_src="$(find "$mount_dir" -maxdepth 2 -type d -name '*.app' | head -n 1)"
  [ -n "$app_src" ] || fail "no .app bundle found in downloaded DMG"
  app_name="$(basename "$app_src")"

  install_root="\${CTX_INSTALL_DIR:-/Applications}"
  if [ -z "\${CTX_INSTALL_DIR:-}" ]; then
    probe_file="$install_root/.ctx-write-probe-$$"
    if ! touch "$probe_file" 2>/dev/null; then
      install_root="$HOME/Applications"
      mkdir -p "$install_root"
    else
      rm -f "$probe_file"
    fi
  else
    mkdir -p "$install_root"
  fi

  target_app="$install_root/$app_name"
  if [ -e "$target_app" ]; then
    rm -rf "$target_app"
  fi
  ditto "$app_src" "$target_app"

  log "Installed $app_name to $target_app"
  if [ "\${CTX_INSTALL_NO_OPEN:-0}" != "1" ]; then
    open "$target_app"
    log "Launched $app_name"
  fi
}

install_linux() {
  local_arch="$1"
  need_cmd python3
  need_cmd chmod
  need_cmd cp

  case "$local_arch" in
    x86_64|amd64) platform="linux-x64" ;;
    aarch64|arm64) platform="linux-arm64" ;;
    *) fail "unsupported Linux architecture: $local_arch" ;;
  esac

  url_path="$(extract_manifest_field_linux "platforms.$platform.desktop.url_path")"
  if [ -z "$url_path" ]; then
    url_path="$(extract_manifest_field_linux "platforms.$platform.appimage.url_path")"
  fi
  [ -n "$url_path" ] || fail "manifest does not contain an AppImage for $platform"

  expected_sha="$(extract_manifest_field_linux "platforms.$platform.desktop.sha256")"
  if [ -z "$expected_sha" ]; then
    expected_sha="$(extract_manifest_field_linux "platforms.$platform.appimage.sha256")"
  fi

  artifact_path="$tmp_dir/ctx-installer.AppImage"
  download_url="$(resolve_download_url "$url_path")"
  log "Downloading $download_url"
  curl -fL --retry 3 --retry-delay 2 "$download_url" -o "$artifact_path"
  verify_artifact_sha "$artifact_path" "$expected_sha"

  install_root="\${CTX_INSTALL_DIR:-$HOME/.local/share/ctx}"
  mkdir -p "$install_root"
  target_appimage="$install_root/ctx.AppImage"
  cp "$artifact_path" "$target_appimage"
  chmod +x "$target_appimage"

  bin_dir="\${CTX_BIN_DIR:-$HOME/.local/bin}"
  mkdir -p "$bin_dir"
  ln -sf "$target_appimage" "$bin_dir/ctx-desktop"

  log "Installed ctx desktop AppImage to $target_appimage"
  log "Created launcher symlink at $bin_dir/ctx-desktop"

  if [ "\${CTX_INSTALL_NO_OPEN:-0}" != "1" ]; then
    if command -v xdg-open >/dev/null 2>&1; then
      xdg-open "$target_appimage" >/dev/null 2>&1 || "$target_appimage" >/dev/null 2>&1 &
    else
      "$target_appimage" >/dev/null 2>&1 &
    fi
    log "Launched ctx desktop"
  fi
}

install_windows() {
  fail "windows support is coming soon!"
}

log "Resolving latest release manifest from $manifest_url"
curl -fsSL "$manifest_url" -o "$manifest_json"

case "$os" in
  Darwin) install_macos "$arch" ;;
  Linux) install_linux "$arch" ;;
  MINGW*|MSYS*|CYGWIN*) install_windows ;;
  *) fail "unsupported operating system: $os" ;;
esac

log "Done."
`;
}
