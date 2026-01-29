#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

log() {
  printf '%s\n' "$*" >&2
}

require_cmd() {
  local cmd="$1"
  if ! command -v "$cmd" >/dev/null 2>&1; then
    log "error: missing required command: $cmd"
    exit 2
  fi
}

require_cmd curl
require_cmd python3

INSTALLER_RS="$ROOT/core/crates/ctx-http/src/installer.rs"
MATRIX_JSON="$ROOT/core/crates/ctx-http/src/provider_matrix.json"

read_const() {
  local name="$1"
  local file="$2"
  local value
  value="$(grep -E "const ${name}" "$file" | head -n 1 | sed -E 's/.*"([^"]+)".*/\1/')"
  if [[ -z "$value" ]]; then
    log "error: failed to read ${name} from ${file}"
    exit 3
  fi
  printf '%s' "$value"
}

NODE_VERSION="$(read_const NODE_VERSION "$INSTALLER_RS")"
PYTHON_VERSION="$(read_const PYTHON_VERSION "$INSTALLER_RS")"
PYTHON_BUILD_TAG="$(read_const PYTHON_BUILD_TAG "$INSTALLER_RS")"

os_raw="$(uname -s)"
case "$os_raw" in
  Linux) os="linux"; matrix_os="linux";;
  Darwin) os="macos"; matrix_os="darwin";;
  MINGW*|MSYS*|CYGWIN*|Windows_NT) os="windows"; matrix_os="windows";;
  *) log "error: unsupported OS: $os_raw"; exit 3;;
esac

arch_raw="$(uname -m)"
case "$arch_raw" in
  x86_64|amd64) arch="x86_64"; matrix_arch="x86_64";;
  aarch64|arm64) arch="aarch64"; matrix_arch="aarch64";;
  *) log "error: unsupported architecture: $arch_raw"; exit 3;;
esac

target_key="${matrix_os}-${matrix_arch}"

case "${os}/${arch}" in
  linux/x86_64) node_target="linux-x64"; python_target="x86_64-unknown-linux-gnu";;
  linux/aarch64) node_target="linux-arm64"; python_target="aarch64-unknown-linux-gnu";;
  macos/x86_64) node_target="darwin-x64"; python_target="x86_64-apple-darwin";;
  macos/aarch64) node_target="darwin-arm64"; python_target="aarch64-apple-darwin";;
  windows/x86_64) node_target="win-x64"; python_target="x86_64-pc-windows-msvc";;
  windows/aarch64) node_target="win-arm64"; python_target="aarch64-pc-windows-msvc";;
  *) log "error: unsupported platform: ${os}/${arch}"; exit 3;;
esac

cache_base="${CTX_BUNDLE_CACHE_DIR:-${XDG_CACHE_HOME:-$HOME/.cache}/ctx/bundles}"
bundle_dir="${CTX_BUNDLE_DIR:-$cache_base/dev}"

mkdir -p "$bundle_dir"

sha256_file() {
  local path="$1"
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$path" | awk '{print $1}'
  else
    shasum -a 256 "$path" | awk '{print $1}'
  fi
}

fetch_file() {
  local url="$1"
  local dest="$2"
  log "download: $url"
  curl -fsSL "$url" -o "$dest"
}

resolve_unique_path() {
  python3 - "$1" "$2" <<'PY'
import os
import sys

root = sys.argv[1]
suffix = sys.argv[2].replace("\\", "/")

matches = []
for dirpath, _, filenames in os.walk(root):
    for name in filenames:
        path = os.path.join(dirpath, name)
        rel = os.path.relpath(path, root).replace("\\", "/")
        if rel.endswith(suffix):
            matches.append(path)

if len(matches) == 1:
    print(matches[0])
    sys.exit(0)
if not matches:
    print(f"error: could not find extracted binary ending with {suffix}", file=sys.stderr)
    sys.exit(2)
print(f"error: multiple extracted binaries match {suffix}", file=sys.stderr)
for m in matches:
    print(m, file=sys.stderr)
sys.exit(3)
PY
}

ensure_node_runtime() {
  local node_folder="node-v${NODE_VERSION}-${node_target}"
  local node_root="$bundle_dir/runtimes/node/${os}/${arch}/${node_folder}"
  local node_bin
  local npm_cli
  if [[ "$os" == "windows" ]]; then
    node_bin="$node_root/node.exe"
    npm_cli="$node_root/node_modules/npm/bin/npm-cli.js"
    archive_ext="zip"
  else
    node_bin="$node_root/bin/node"
    npm_cli="$node_root/lib/node_modules/npm/bin/npm-cli.js"
    archive_ext="tar.gz"
  fi

  if [[ -f "$node_bin" && -f "$npm_cli" ]]; then
    return
  fi

  local dest_dir
  dest_dir="$(dirname "$node_root")"
  mkdir -p "$dest_dir"
  local tmp
  tmp="$(mktemp -p "$dest_dir" "node-${node_folder}.XXXXXX.${archive_ext}")"
  local url="https://nodejs.org/dist/v${NODE_VERSION}/${node_folder}.${archive_ext}"
  fetch_file "$url" "$tmp"

  local extract_dir
  extract_dir="$(mktemp -d "${dest_dir}/node-${node_folder}.extract.XXXXXX")"
  if [[ "$archive_ext" == "zip" ]]; then
    require_cmd unzip
    unzip -q "$tmp" -d "$extract_dir"
  else
    require_cmd tar
    tar -xzf "$tmp" -C "$extract_dir"
  fi

  local extracted="$extract_dir/$node_folder"
  if [[ ! -d "$extracted" ]]; then
    log "error: node extraction failed (missing $node_folder in $extract_dir)"
    exit 4
  fi

  rm -rf "$node_root"
  mv "$extracted" "$node_root"
  rm -rf "$extract_dir" "$tmp"

  if [[ ! -f "$node_bin" || ! -f "$npm_cli" ]]; then
    log "error: node runtime incomplete after extract"
    exit 4
  fi
}

ensure_python_runtime() {
  local py_folder="cpython-${PYTHON_VERSION}+${PYTHON_BUILD_TAG}-${python_target}"
  local py_root="$bundle_dir/runtimes/python/${os}/${arch}/${py_folder}"
  local py_bin
  if [[ "$os" == "windows" ]]; then
    py_bin="$py_root/python.exe"
  else
    py_bin="$py_root/bin/python3"
    if [[ ! -f "$py_bin" ]]; then
      py_bin="$py_root/bin/python"
    fi
  fi

  if [[ -f "$py_bin" ]]; then
    return
  fi

  local dest_dir
  dest_dir="$(dirname "$py_root")"
  mkdir -p "$dest_dir"
  local asset="cpython-${PYTHON_VERSION}+${PYTHON_BUILD_TAG}-${python_target}-install_only.tar.gz"
  local url="https://github.com/indygreg/python-build-standalone/releases/download/${PYTHON_BUILD_TAG}/${asset}"
  local tmp
  tmp="$(mktemp -p "$dest_dir" "python-${py_folder}.XXXXXX.tar.gz")"
  fetch_file "$url" "$tmp"

  require_cmd tar
  local extract_dir
  extract_dir="$(mktemp -d "${dest_dir}/python-${py_folder}.extract.XXXXXX")"
  tar -xzf "$tmp" -C "$extract_dir"

  local extracted="$extract_dir/python"
  if [[ ! -d "$extracted" ]]; then
    log "error: python extraction failed (missing python/ in $extract_dir)"
    exit 4
  fi

  rm -rf "$py_root"
  mv "$extracted" "$py_root"
  rm -rf "$extract_dir" "$tmp"

  if [[ "$os" == "windows" ]]; then
    py_bin="$py_root/python.exe"
  else
    py_bin="$py_root/bin/python3"
    if [[ ! -f "$py_bin" ]]; then
      py_bin="$py_root/bin/python"
    fi
  fi
  if [[ ! -f "$py_bin" ]]; then
    log "error: python runtime incomplete after extract"
    exit 4
  fi
}

ensure_node_runtime
ensure_python_runtime

providers_src="$(mktemp /tmp/ctx-bundle-providers.XXXXXX)"
python3 - "$MATRIX_JSON" "$target_key" > "$providers_src" <<'PY'
import json
import sys
from pathlib import Path

path = Path(sys.argv[1])
target = sys.argv[2]

data = json.loads(path.read_text())
for provider in data.get("providers", []):
    mi = provider.get("managed_install")
    if not mi or mi.get("kind") != "archive":
        continue
    target_entry = mi.get("targets", {}).get(target)
    if not target_entry:
        continue
    args = mi.get("args") or []
    line = "\t".join(
        [
            provider.get("id", ""),
            mi.get("version", ""),
            target_entry.get("url", ""),
            target_entry.get("archive", ""),
            target_entry.get("bin_path", ""),
            json.dumps(args, separators=(",", ":")),
        ]
    )
    print(line)
PY

providers_out="$(mktemp /tmp/ctx-bundle-providers-out.XXXXXX)"

while IFS=$'\t' read -r provider_id version url archive bin_path args_json; do
  if [[ -z "$provider_id" || -z "$version" || -z "$url" ]]; then
    continue
  fi
  provider_root="$bundle_dir/providers/${provider_id}/${os}/${arch}"
  version_marker="$provider_root/.version"
  if [[ -f "$version_marker" ]]; then
    if [[ "$(cat "$version_marker" 2>/dev/null || true)" != "$version" ]]; then
      rm -rf "$provider_root"
    fi
  fi

  if [[ ! -d "$provider_root" ]]; then
    mkdir -p "$provider_root"
    tmp_file="$(mktemp -p "$provider_root" "${provider_id}.XXXXXX")"
    fetch_file "$url" "$tmp_file"
    case "$archive" in
      none)
        dest="$provider_root/$bin_path"
        mkdir -p "$(dirname "$dest")"
        mv "$tmp_file" "$dest"
        ;;
      tar_gz)
        require_cmd tar
        tar -xzf "$tmp_file" -C "$provider_root"
        rm -f "$tmp_file"
        ;;
      tar_bz2)
        require_cmd tar
        tar -xjf "$tmp_file" -C "$provider_root"
        rm -f "$tmp_file"
        ;;
      zip)
        require_cmd unzip
        unzip -q "$tmp_file" -d "$provider_root"
        rm -f "$tmp_file"
        ;;
      *)
        log "error: unsupported archive type '$archive' for $provider_id"
        exit 5
        ;;
    esac
    echo "$version" > "$version_marker"
  fi

  command_path="$provider_root/$bin_path"
  if [[ ! -f "$command_path" ]]; then
    command_path="$(resolve_unique_path "$provider_root" "$bin_path")"
  fi
  if [[ "$os" != "windows" ]]; then
    chmod +x "$command_path" || true
  fi

  if [[ "$command_path" != "$bundle_dir"/* ]]; then
    log "error: resolved command path outside bundle dir: $command_path"
    exit 6
  fi

  rel_command="${command_path#"$bundle_dir/"}"
  sha256="$(sha256_file "$command_path")"
  protocol="acp"
  if [[ "$provider_id" == *"-crp" ]]; then
    protocol="crp"
  fi

  PROVIDER_ID="$provider_id" \
  PROVIDER_VERSION="$version" \
  PROVIDER_PROTOCOL="$protocol" \
  PROVIDER_OS="$os" \
  PROVIDER_ARCH="$arch" \
  PROVIDER_SHA256="$sha256" \
  PROVIDER_COMMAND="$rel_command" \
  PROVIDER_ARGS_JSON="$args_json" \
  python3 - <<'PY' >> "$providers_out"
import json
import os

entry = {
    "id": os.environ["PROVIDER_ID"],
    "protocol": os.environ["PROVIDER_PROTOCOL"],
    "version": os.environ["PROVIDER_VERSION"],
    "os": os.environ["PROVIDER_OS"],
    "arch": os.environ["PROVIDER_ARCH"],
    "sha256": os.environ["PROVIDER_SHA256"],
    "command": os.environ["PROVIDER_COMMAND"],
    "args": json.loads(os.environ["PROVIDER_ARGS_JSON"] or "[]"),
}
print(json.dumps(entry, separators=(",", ":")))
PY

  unset PROVIDER_ID PROVIDER_VERSION PROVIDER_PROTOCOL PROVIDER_OS PROVIDER_ARCH PROVIDER_SHA256 PROVIDER_COMMAND PROVIDER_ARGS_JSON

done < "$providers_src"

runtimes_out="$(mktemp /tmp/ctx-bundle-runtimes-out.XXXXXX)"

node_root_rel="runtimes/node/${os}/${arch}/node-v${NODE_VERSION}-${node_target}"
if [[ "$os" == "windows" ]]; then
  node_bin_rel="node.exe"
  npm_cli_rel="node_modules/npm/bin/npm-cli.js"
else
  node_bin_rel="bin/node"
  npm_cli_rel="lib/node_modules/npm/bin/npm-cli.js"
fi
node_root="$bundle_dir/$node_root_rel"
node_bin="$node_root/$node_bin_rel"
node_sha="$(sha256_file "$node_bin")"
NODE_VERSION_ENV="$NODE_VERSION" \
NODE_OS_ENV="$os" \
NODE_ARCH_ENV="$arch" \
NODE_SHA_ENV="$node_sha" \
NODE_ROOT_REL_ENV="$node_root_rel" \
NODE_BIN_REL_ENV="$node_bin_rel" \
NODE_NPM_REL_ENV="$npm_cli_rel" \
python3 - <<'PY' >> "$runtimes_out"
import json
import os

entry = {
    "id": "node",
    "version": os.environ["NODE_VERSION_ENV"],
    "os": os.environ["NODE_OS_ENV"],
    "arch": os.environ["NODE_ARCH_ENV"],
    "sha256": os.environ["NODE_SHA_ENV"],
    "root": os.environ["NODE_ROOT_REL_ENV"],
    "bin": os.environ["NODE_BIN_REL_ENV"],
    "npm_cli": os.environ["NODE_NPM_REL_ENV"],
}
print(json.dumps(entry, separators=(",", ":")))
PY

python_root_rel="runtimes/python/${os}/${arch}/cpython-${PYTHON_VERSION}+${PYTHON_BUILD_TAG}-${python_target}"
if [[ "$os" == "windows" ]]; then
  python_bin_rel="python.exe"
else
  python_bin_rel="bin/python3"
  if [[ ! -f "$bundle_dir/$python_root_rel/$python_bin_rel" ]]; then
    python_bin_rel="bin/python"
  fi
fi
python_root="$bundle_dir/$python_root_rel"
python_bin="$python_root/$python_bin_rel"
python_sha="$(sha256_file "$python_bin")"
PYTHON_VERSION_ENV="$PYTHON_VERSION" \
PYTHON_OS_ENV="$os" \
PYTHON_ARCH_ENV="$arch" \
PYTHON_SHA_ENV="$python_sha" \
PYTHON_ROOT_REL_ENV="$python_root_rel" \
PYTHON_BIN_REL_ENV="$python_bin_rel" \
python3 - <<'PY' >> "$runtimes_out"
import json
import os

entry = {
    "id": "python",
    "version": os.environ["PYTHON_VERSION_ENV"],
    "os": os.environ["PYTHON_OS_ENV"],
    "arch": os.environ["PYTHON_ARCH_ENV"],
    "sha256": os.environ["PYTHON_SHA_ENV"],
    "root": os.environ["PYTHON_ROOT_REL_ENV"],
    "bin": os.environ["PYTHON_BIN_REL_ENV"],
}
print(json.dumps(entry, separators=(",", ":")))
PY

manifest_path="$bundle_dir/manifest.json"
GENERATED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
PROVIDERS_OUT="$providers_out" \
RUNTIMES_OUT="$runtimes_out" \
GENERATED_AT_ENV="$GENERATED_AT" \
MANIFEST_PATH="$manifest_path" \
python3 - <<'PY'
import json
import os
from pathlib import Path

providers_path = Path(os.environ["PROVIDERS_OUT"])
runtimes_path = Path(os.environ["RUNTIMES_OUT"])
manifest_path = Path(os.environ.get("MANIFEST_PATH", "manifest.json"))

providers = []
if providers_path.exists():
    for line in providers_path.read_text().splitlines():
        if line.strip():
            providers.append(json.loads(line))

runtimes = []
if runtimes_path.exists():
    for line in runtimes_path.read_text().splitlines():
        if line.strip():
            runtimes.append(json.loads(line))

manifest = {
    "version": 1,
    "generated_at": os.environ["GENERATED_AT_ENV"],
    "providers": providers,
    "runtimes": runtimes,
}

manifest_path.write_text(json.dumps(manifest, indent=2))
PY

rm -f "$providers_src" "$providers_out" "$runtimes_out" || true

echo "$bundle_dir"
