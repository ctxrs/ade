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

is_truthy() {
  case "${1:-}" in
    1|true|TRUE|yes|YES|on|ON) return 0;;
    *) return 1;;
  esac
}

is_falsy() {
  case "${1:-}" in
    0|false|FALSE|no|NO|off|OFF) return 0;;
    *) return 1;;
  esac
}

require_cmd curl

PYTHON_HOST_CMD=()
if command -v python3 >/dev/null 2>&1; then
  PYTHON_HOST_CMD=(python3)
elif command -v python >/dev/null 2>&1; then
  PYTHON_HOST_CMD=(python)
elif command -v py >/dev/null 2>&1; then
  PYTHON_HOST_CMD=(py -3)
else
  log "error: missing required command: python3 (or python/py)"
  exit 2
fi

run_python() {
  "${PYTHON_HOST_CMD[@]}" "$@"
}

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
PODMAN_VERSION="${PODMAN_VERSION:-}"
PODMAN_ARCHIVE_URL="${PODMAN_ARCHIVE_URL:-}"
PODMAN_ARCHIVE_PATH="${PODMAN_ARCHIVE_PATH:-}"
PODMAN_BIN_REL="${PODMAN_BIN_REL:-}"
PODMAN_EXTRACT_SUBDIR="${PODMAN_EXTRACT_SUBDIR:-}"

bundle_os="${CTX_BUNDLE_OS:-${CTX_BUNDLE_TARGET_OS:-}}"
bundle_arch="${CTX_BUNDLE_ARCH:-${CTX_BUNDLE_TARGET_ARCH:-}}"

if [[ -n "$bundle_os" ]]; then
  case "$bundle_os" in
    linux|Linux) os="linux"; matrix_os="linux";;
    macos|darwin|Darwin) os="macos"; matrix_os="darwin";;
    windows|win32|Windows_NT|MINGW*|MSYS*|CYGWIN*) os="windows"; matrix_os="windows";;
    *) log "error: unsupported CTX_BUNDLE_OS/CTX_BUNDLE_TARGET_OS: $bundle_os"; exit 3;;
  esac
else
  os_raw="$(uname -s)"
  case "$os_raw" in
    Linux) os="linux"; matrix_os="linux";;
    Darwin) os="macos"; matrix_os="darwin";;
    MINGW*|MSYS*|CYGWIN*|Windows_NT) os="windows"; matrix_os="windows";;
    *) log "error: unsupported OS: $os_raw"; exit 3;;
  esac
fi

if [[ -n "$bundle_arch" ]]; then
  case "$bundle_arch" in
    x86_64|amd64) arch="x86_64"; matrix_arch="x86_64";;
    aarch64|arm64) arch="aarch64"; matrix_arch="aarch64";;
    *) log "error: unsupported CTX_BUNDLE_ARCH/CTX_BUNDLE_TARGET_ARCH: $bundle_arch"; exit 3;;
  esac
else
  arch_raw="$(uname -m)"
  case "$arch_raw" in
    x86_64|amd64) arch="x86_64"; matrix_arch="x86_64";;
    aarch64|arm64) arch="aarch64"; matrix_arch="aarch64";;
    *) log "error: unsupported architecture: $arch_raw"; exit 3;;
  esac
fi

BIN_EXT=""
if [[ "$os" == "windows" ]]; then
  BIN_EXT=".exe"
fi

target_key="${matrix_os}-${matrix_arch}"

case "${os}/${arch}" in
  linux/x86_64) node_target="linux-x64"; python_target="x86_64-unknown-linux-gnu"; rust_target="x86_64-unknown-linux-gnu";;
  linux/aarch64) node_target="linux-arm64"; python_target="aarch64-unknown-linux-gnu"; rust_target="aarch64-unknown-linux-gnu";;
  macos/x86_64) node_target="darwin-x64"; python_target="x86_64-apple-darwin"; rust_target="x86_64-apple-darwin";;
  macos/aarch64) node_target="darwin-arm64"; python_target="aarch64-apple-darwin"; rust_target="aarch64-apple-darwin";;
  windows/x86_64) node_target="win-x64"; python_target="x86_64-pc-windows-msvc"; rust_target="x86_64-pc-windows-msvc";;
  windows/aarch64) node_target="win-arm64"; python_target="aarch64-pc-windows-msvc"; rust_target="aarch64-pc-windows-msvc";;
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
  run_python - "$1" "$2" <<'PY'
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

resolve_npm_entrypoint() {
  run_python - "$1" "$2" "$3" <<'PY'
import json
import os
import sys

root, package, entry = sys.argv[1:4]
pkg_dir = os.path.join(root, "node_modules", package)
pkg_json = os.path.join(pkg_dir, "package.json")

if not os.path.isfile(pkg_json):
    print(f"error: missing package.json for {package} in {pkg_dir}", file=sys.stderr)
    sys.exit(2)

with open(pkg_json, "r", encoding="utf-8") as fh:
    data = json.load(fh)

bin_field = data.get("bin")
rel = None
if isinstance(bin_field, str):
    rel = bin_field
elif isinstance(bin_field, dict):
    bin_name = os.path.basename(entry)
    rel = bin_field.get(bin_name)
    if rel is None and bin_field:
        rel = next(iter(bin_field.values()))

if not rel:
    print(f"error: unable to resolve npm entrypoint for {package}", file=sys.stderr)
    sys.exit(2)

resolved = os.path.normpath(os.path.join(pkg_dir, rel))
print(resolved)
PY
}

BRIDGE_DIR="${CTX_BUNDLE_BRIDGE_DIR:-$ROOT/external-harnesses/acp-crp-bridge}"
BRIDGE_BIN="acp-crp-bridge"
LOCAL_ADAPTERS_DIR="${CTX_BUNDLE_ADAPTERS_DIR:-$ROOT/harness-adapters}"
LOCAL_ADAPTER_MODE="${CTX_BUNDLE_LOCAL_ADAPTERS:-auto}"
BUILD_LOCAL_ADAPTERS="${CTX_BUNDLE_BUILD_LOCAL_ADAPTERS:-0}"

declare -A LOCAL_ADAPTER_DIR=(
  ["amp"]="amp-acp"
  ["droid"]="droid-acp"
  ["copilot"]="copilot-cli-acp"
  ["kiro"]="kiro-acp"
  ["rovo"]="rovo-dev-acp"
  ["cody"]="cody-acp"
)

declare -A LOCAL_ADAPTER_BIN=(
  ["droid"]="droid-acp"
  ["copilot"]="copilot-cli-acp"
  ["kiro"]="kiro-acp"
  ["rovo"]="rovo-dev-acp"
  ["cody"]="cody-acp"
)

get_matrix_version() {
  local provider_id="$1"
  run_python - "$MATRIX_JSON" "$provider_id" <<'PY'
import json
import sys

path = sys.argv[1]
provider_id = sys.argv[2]
data = json.loads(open(path, "r", encoding="utf-8").read())
for provider in data.get("providers", []):
    if provider.get("id") == provider_id:
        mi = provider.get("managed_install") or {}
        print(mi.get("version") or "")
        sys.exit(0)
print("")
PY
}

local_adapter_binary_path() {
  local provider_id="$1"
  local dir="${LOCAL_ADAPTER_DIR[$provider_id]}"
  local bin="${LOCAL_ADAPTER_BIN[$provider_id]}"
  if [[ -z "$dir" || -z "$bin" ]]; then
    return 1
  fi
  if [[ -n "${CARGO_TARGET_DIR:-}" ]]; then
    if [[ -n "${rust_target:-}" ]]; then
      printf '%s' "$CARGO_TARGET_DIR/$rust_target/release/${bin}${BIN_EXT}"
    else
      printf '%s' "$CARGO_TARGET_DIR/release/${bin}${BIN_EXT}"
    fi
    return 0
  fi
  printf '%s' "$LOCAL_ADAPTERS_DIR/$dir/target/$rust_target/release/${bin}${BIN_EXT}"
}

local_bridge_binary_path() {
  if [[ -n "${CARGO_TARGET_DIR:-}" ]]; then
    if [[ -n "${rust_target:-}" ]]; then
      printf '%s' "$CARGO_TARGET_DIR/$rust_target/release/${BRIDGE_BIN}${BIN_EXT}"
    else
      printf '%s' "$CARGO_TARGET_DIR/release/${BRIDGE_BIN}${BIN_EXT}"
    fi
    return 0
  fi
  printf '%s' "$BRIDGE_DIR/target/$rust_target/release/${BRIDGE_BIN}${BIN_EXT}"
}

require_bridge_binary() {
  if [[ ! -d "$BRIDGE_DIR" ]]; then
    log "error: missing acp-crp-bridge source dir at $BRIDGE_DIR"
    exit 5
  fi
  local bridge_out
  bridge_out="$(local_bridge_binary_path)"
  if [[ ! -f "$bridge_out" ]]; then
    if is_truthy "$BUILD_LOCAL_ADAPTERS"; then
      require_cmd cargo
      (cd "$BRIDGE_DIR" && cargo build --release --target "$rust_target")
    fi
  fi
  if [[ ! -f "$bridge_out" ]]; then
    log "error: missing acp-crp-bridge binary at $bridge_out"
    exit 5
  fi
}

local_adapter_amp_entrypoint() {
  printf '%s' "$LOCAL_ADAPTERS_DIR/${LOCAL_ADAPTER_DIR[amp]}/dist/bin/amp-acp.js"
}

build_local_adapters() {
  if [[ ! -d "$LOCAL_ADAPTERS_DIR" ]]; then
    log "error: local adapters dir missing: $LOCAL_ADAPTERS_DIR"
    exit 4
  fi

  local amp_js
  amp_js="$(local_adapter_amp_entrypoint)"
  if [[ ! -f "$amp_js" ]]; then
    if [[ -d "$LOCAL_ADAPTERS_DIR/${LOCAL_ADAPTER_DIR[amp]}" ]]; then
      require_cmd npm
      (cd "$LOCAL_ADAPTERS_DIR/${LOCAL_ADAPTER_DIR[amp]}" && npm install)
      (cd "$LOCAL_ADAPTERS_DIR/${LOCAL_ADAPTER_DIR[amp]}" && npm run build)
    fi
  fi

  local id
  for id in droid copilot kiro rovo cody; do
    local dir="${LOCAL_ADAPTER_DIR[$id]}"
    local bin="${LOCAL_ADAPTER_BIN[$id]}"
    if [[ -z "$dir" || -z "$bin" ]]; then
      continue
    fi
    local out
    out="$(local_adapter_binary_path "$id" || true)"
    if [[ -f "$out" ]]; then
      continue
    fi
    require_cmd cargo
    (cd "$LOCAL_ADAPTERS_DIR/$dir" && cargo build --release --target "$rust_target")
  done

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

resolve_podman_root() {
  local extract_dir="$1"
  if [[ -n "$PODMAN_EXTRACT_SUBDIR" ]]; then
    printf '%s' "${extract_dir}/${PODMAN_EXTRACT_SUBDIR}"
    return
  fi
  local entries
  entries=("$extract_dir"/*)
  if [[ ${#entries[@]} -eq 1 && -d "${entries[0]}" ]]; then
    printf '%s' "${entries[0]}"
    return
  fi
  printf '%s' "$extract_dir"
}

ensure_podman_runtime() {
  if [[ "${CTX_BUNDLE_PODMAN:-0}" != "1" ]]; then
    return
  fi
  if [[ -z "$PODMAN_VERSION" ]]; then
    log "error: PODMAN_VERSION is required when CTX_BUNDLE_PODMAN=1"
    exit 3
  fi

  local podman_root="runtimes/podman/${os}/${arch}/podman-${PODMAN_VERSION}"
  local podman_root_abs="$bundle_dir/$podman_root"
  local podman_bin_rel
  if [[ -n "$PODMAN_BIN_REL" ]]; then
    podman_bin_rel="$PODMAN_BIN_REL"
  elif [[ "$os" == "windows" ]]; then
    podman_bin_rel="podman.exe"
  else
    podman_bin_rel="bin/podman"
  fi
  local podman_bin_abs="$podman_root_abs/$podman_bin_rel"

  if [[ -f "$podman_bin_abs" ]]; then
    return
  fi

  local archive_path=""
  if [[ -n "$PODMAN_ARCHIVE_PATH" ]]; then
    archive_path="$PODMAN_ARCHIVE_PATH"
  elif [[ -n "$PODMAN_ARCHIVE_URL" ]]; then
    local dest_dir
    dest_dir="$(dirname "$podman_root_abs")"
    mkdir -p "$dest_dir"
    local ext="tar.gz"
    if [[ "$PODMAN_ARCHIVE_URL" == *.zip ]]; then
      ext="zip"
    elif [[ "$PODMAN_ARCHIVE_URL" == *.tar ]]; then
      ext="tar"
    elif [[ "$PODMAN_ARCHIVE_URL" == *.tgz ]]; then
      ext="tgz"
    fi
    archive_path="$(mktemp -p "$dest_dir" "podman-${PODMAN_VERSION}.XXXXXX.${ext}")"
    fetch_file "$PODMAN_ARCHIVE_URL" "$archive_path"
  else
    log "error: PODMAN_ARCHIVE_URL or PODMAN_ARCHIVE_PATH is required when CTX_BUNDLE_PODMAN=1"
    exit 3
  fi

  local extract_dir
  extract_dir="$(mktemp -d "$(dirname "$podman_root_abs")/podman-${PODMAN_VERSION}.extract.XXXXXX")"
  case "$archive_path" in
    *.zip)
      require_cmd unzip
      unzip -q "$archive_path" -d "$extract_dir"
      ;;
    *.tar)
      require_cmd tar
      tar -xf "$archive_path" -C "$extract_dir"
      ;;
    *.tgz|*.tar.gz)
      require_cmd tar
      tar -xzf "$archive_path" -C "$extract_dir"
      ;;
    *)
      log "error: unsupported podman archive type: $archive_path"
      exit 4
      ;;
  esac

  local extracted_root
  extracted_root="$(resolve_podman_root "$extract_dir")"
  if [[ ! -d "$extracted_root" ]]; then
    log "error: podman extraction failed (missing root at $extracted_root)"
    exit 4
  fi

  rm -rf "$podman_root_abs"
  mv "$extracted_root" "$podman_root_abs"
  rm -rf "$extract_dir"
  if [[ "$archive_path" != "$PODMAN_ARCHIVE_PATH" ]]; then
    rm -f "$archive_path"
  fi

  if [[ ! -f "$podman_bin_abs" ]]; then
    log "error: podman runtime incomplete after extract (missing $podman_bin_rel)"
    exit 4
  fi
}

venv_bin_dir() {
  local venv_dir="$1"
  if [[ "$os" == "windows" ]]; then
    printf '%s' "$venv_dir/Scripts"
  else
    printf '%s' "$venv_dir/bin"
  fi
}

venv_exe() {
  local venv_dir="$1"
  local name="$2"
  local bin_dir
  bin_dir="$(venv_bin_dir "$venv_dir")"
  if [[ "$os" == "windows" ]]; then
    printf '%s' "$bin_dir/${name}.exe"
  else
    printf '%s' "$bin_dir/$name"
  fi
}

ensure_venv_pip() {
  local venv_python="$1"
  if "$venv_python" -m pip --version >/dev/null 2>&1; then
    return
  fi
  "$venv_python" -m ensurepip --upgrade >/dev/null
}

npm_install_bundle() {
  local install_dir="$1"
  local package_spec="$2"
  local cache_dir="$install_dir/.npm-cache"
  mkdir -p "$cache_dir"

  local node_bin_dir
  node_bin_dir="$(dirname "$node_bin")"
  local path_sep=":"
  if [[ "$os" == "windows" ]]; then
    path_sep=";"
  fi

  PATH="${node_bin_dir}${path_sep}${PATH:-}" \
  npm_config_update_notifier="false" \
  npm_config_fund="false" \
  npm_config_audit="false" \
  npm_config_progress="false" \
  npm_config_cache="$cache_dir" \
  "$node_bin" "$npm_cli" install --prefix "$install_dir" --no-audit --no-fund --silent "$package_spec"
}

ensure_node_runtime
ensure_python_runtime
ensure_podman_runtime
require_bridge_binary

if is_truthy "$BUILD_LOCAL_ADAPTERS"; then
  if is_falsy "$LOCAL_ADAPTER_MODE"; then
    log "warn: CTX_BUNDLE_BUILD_LOCAL_ADAPTERS set but CTX_BUNDLE_LOCAL_ADAPTERS=off"
  fi
  build_local_adapters
fi

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
npm_cli="$node_root/$npm_cli_rel"

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

podman_root_rel=""
podman_bin_rel=""
podman_root=""
podman_bin=""
if [[ "${CTX_BUNDLE_PODMAN:-0}" == "1" ]]; then
  podman_root_rel="runtimes/podman/${os}/${arch}/podman-${PODMAN_VERSION}"
  if [[ -n "$PODMAN_BIN_REL" ]]; then
    podman_bin_rel="$PODMAN_BIN_REL"
  elif [[ "$os" == "windows" ]]; then
    podman_bin_rel="podman.exe"
  else
    podman_bin_rel="bin/podman"
  fi
  podman_root="$bundle_dir/$podman_root_rel"
  podman_bin="$podman_root/$podman_bin_rel"
fi

providers_src="$(mktemp /tmp/ctx-bundle-providers.XXXXXX)"
run_python - "$MATRIX_JSON" "$target_key" "$ROOT/core/crates/ctx-http/Cargo.toml" > "$providers_src" <<'PY'
import json
import re
import sys
from pathlib import Path

path = Path(sys.argv[1])
target = sys.argv[2]
cargo = Path(sys.argv[3])
sep = "\x1f"

def parse_version_loose(raw: str):
    if not raw:
        return None
    trimmed = raw.strip().lstrip("v")
    if not trimmed:
        return None
    if trimmed.count(".") == 1:
        trimmed = f"{trimmed}.0"
    match = re.match(r"([0-9]+(?:\\.[0-9]+)*)", trimmed)
    if not match:
        return None
    try:
        return tuple(int(part) for part in match.group(1).split("."))
    except ValueError:
        return None

def release_matches_context(release, ctx_version):
    if ctx_version is None:
        return True
    min_v = parse_version_loose(release.get("context_min", ""))
    if min_v and ctx_version < min_v:
        return False
    max_v = parse_version_loose(release.get("context_max", ""))
    if max_v and ctx_version > max_v:
        return False
    return True

def select_latest_release(candidates):
    best = None
    best_v = None
    for release in candidates:
        parsed = parse_version_loose(release.get("version", ""))
        if parsed is None:
            continue
        if best_v is None or parsed > best_v:
            best_v = parsed
            best = release
    if best is not None:
        return best
    return candidates[-1] if candidates else None

ctx_version = None
try:
    for line in cargo.read_text().splitlines():
        if line.strip().startswith("version"):
            _, raw = line.split("=", 1)
            ctx_version = parse_version_loose(raw.strip().strip('"'))
            break
except FileNotFoundError:
    ctx_version = None

data = json.loads(path.read_text())
for provider in data.get("providers", []):
    mi = provider.get("managed_install") or {}
    kind = mi.get("kind")
    if not kind:
        continue
    args = mi.get("args") or []
    if kind == "archive":
        target_entry = mi.get("targets", {}).get(target)
        if not target_entry:
            continue
        line = sep.join(
            [
                provider.get("id", ""),
                "archive",
                mi.get("version", ""),
                target_entry.get("url", ""),
                target_entry.get("archive", ""),
                target_entry.get("bin_path", ""),
                "",
                "",
                json.dumps(args, separators=(",", ":")),
            ]
        )
        print(line)
    elif kind == "npm":
        releases = [
            r for r in provider.get("releases", []) if r.get("status") == "supported"
        ]
        releases = [r for r in releases if release_matches_context(r, ctx_version)]
        release = select_latest_release(releases)
        version = release.get("version", "") if release else ""
        line = sep.join(
            [
                provider.get("id", ""),
                "npm",
                version,
                "",
                "",
                "",
                mi.get("package", ""),
                mi.get("entrypoint", ""),
                json.dumps(args, separators=(",", ":")),
            ]
        )
        print(line)
    elif kind == "python":
        line = sep.join(
            [
                provider.get("id", ""),
                "python",
                mi.get("version", ""),
                "",
                "",
                "",
                mi.get("package", ""),
                mi.get("entrypoint", ""),
                json.dumps(args, separators=(",", ":")),
            ]
        )
        print(line)
PY

local_providers_src="$(mktemp /tmp/ctx-bundle-local-providers.XXXXXX)"
local_ids=()

add_local_provider() {
  local provider_id="$1"
  local kind="$2"
  local version="$3"
  local source_path="$4"
  local bin_path="$5"
  local args_json="${6:-[]}"
  local sep=$'\x1f'
  printf '%s%s%s%s%s%s%s%s%s%s%s%s%s%s%s%s%s\n' \
    "$provider_id" "$sep" "$kind" "$sep" "$version" "$sep" \
    "$source_path" "$sep" "" "$sep" "$bin_path" "$sep" "" "$sep" "" "$sep" \
    "$args_json" >> "$local_providers_src"
  local_ids+=("$provider_id")
}

bridge_src="$(local_bridge_binary_path)"
add_local_provider "acp-crp-bridge" "local-bin" "local" "$bridge_src" "$(basename "$bridge_src")" "[]"

if ! is_falsy "$LOCAL_ADAPTER_MODE"; then
  local_adapter_required=0
  if is_truthy "$LOCAL_ADAPTER_MODE"; then
    local_adapter_required=1
  fi

  adapter_version_override="${CTX_BUNDLE_ADAPTER_VERSION:-}"
  for id in amp droid copilot kiro rovo cody; do
    version="$adapter_version_override"
    if [[ -z "$version" ]]; then
      version="$(get_matrix_version "$id")"
    fi
    if [[ -z "$version" ]]; then
      version="local"
    fi

    if [[ "$id" == "amp" ]]; then
      src="$(local_adapter_amp_entrypoint)"
      if [[ -f "$src" ]]; then
        add_local_provider "$id" "local-node" "$version" "$src" "amp-acp.js" "[]"
      elif [[ "$local_adapter_required" == "1" ]]; then
        log "error: missing amp adapter entrypoint at $src"
        exit 5
      fi
      continue
    fi

    src="$(local_adapter_binary_path "$id" || true)"
    if [[ -f "$src" ]]; then
      add_local_provider "$id" "local-bin" "$version" "$src" "$(basename "$src")" "[]"
    elif [[ "$local_adapter_required" == "1" ]]; then
      log "error: missing local adapter binary for $id at $src"
      exit 5
    fi
  done
fi

if [[ ${#local_ids[@]} -gt 0 ]]; then
  ids_csv="$(IFS=,; echo "${local_ids[*]}")"
  run_python - "$providers_src" "$ids_csv" <<'PY'
import sys

path = sys.argv[1]
ids = {v for v in sys.argv[2].split(",") if v}
sep = "\x1f"

lines = []
with open(path, "r", encoding="utf-8") as fh:
    for line in fh:
        if not line.strip():
            continue
        provider_id = line.split(sep, 1)[0]
        if provider_id in ids:
            continue
        lines.append(line)

with open(path, "w", encoding="utf-8") as fh:
    fh.writelines(lines)
PY
  cat "$local_providers_src" >> "$providers_src"
fi

providers_out="$(mktemp /tmp/ctx-bundle-providers-out.XXXXXX)"
skip_providers_raw="${CTX_BUNDLE_SKIP_PROVIDERS:-}"
skip_providers_raw="${skip_providers_raw// /}"

while IFS=$'\x1f' read -r provider_id kind version url archive bin_path package entrypoint args_json; do
  if [[ -z "$provider_id" || -z "$kind" ]]; then
    continue
  fi
  if [[ -n "$skip_providers_raw" ]]; then
    if [[ ",$skip_providers_raw," == *",$provider_id,"* ]]; then
      continue
    fi
  fi

  provider_root="$bundle_dir/providers/${provider_id}/${os}/${arch}"
  version_marker="$provider_root/.version"

  case "$kind" in
    local-bin)
      if [[ -z "$url" ]]; then
        log "error: missing local adapter path for $provider_id"
        exit 5
      fi
      if [[ -z "$bin_path" ]]; then
        bin_path="$(basename "$url")"
      fi
      mkdir -p "$provider_root"
      dest="$provider_root/$bin_path"
      mkdir -p "$(dirname "$dest")"
      cp "$url" "$dest"
      if [[ "$os" != "windows" ]]; then
        chmod +x "$dest" || true
      fi
      echo "$version" > "$version_marker"
      command_path="$dest"
      ;;
    local-node)
      if [[ -z "$url" ]]; then
        log "error: missing local adapter entrypoint for $provider_id"
        exit 5
      fi
      if [[ -z "$bin_path" ]]; then
        bin_path="$(basename "$url")"
      fi
      mkdir -p "$provider_root"
      dest="$provider_root/$bin_path"
      mkdir -p "$(dirname "$dest")"
      cp "$url" "$dest"
      echo "$version" > "$version_marker"
      command_path="$node_bin"
      entrypoint_rel="${dest#"$bundle_dir/"}"
      args_json="$(PROVIDER_ENTRYPOINT="$entrypoint_rel" PROVIDER_ARGS_JSON="$args_json" run_python - <<'PY'
import json
import os

args = [os.environ["PROVIDER_ENTRYPOINT"]]
extra = json.loads(os.environ["PROVIDER_ARGS_JSON"] or "[]")
args.extend(extra)
print(json.dumps(args, separators=(",", ":")))
PY
)"
      ;;
    archive)
      if [[ -z "$version" || -z "$url" ]]; then
        continue
      fi
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
      ;;
    npm)
      if [[ -z "$version" || -z "$package" || -z "$entrypoint" ]]; then
        log "error: missing npm metadata for $provider_id"
        exit 5
      fi

      entrypoint_path="$provider_root/$entrypoint"
      if [[ -f "$version_marker" ]]; then
        if [[ "$(cat "$version_marker" 2>/dev/null || true)" != "$version" ]]; then
          rm -rf "$provider_root"
        elif [[ ! -f "$entrypoint_path" ]]; then
          resolved_entrypoint="$(resolve_npm_entrypoint "$provider_root" "$package" "$entrypoint" || true)"
          if [[ -n "$resolved_entrypoint" && -f "$resolved_entrypoint" ]]; then
            entrypoint_path="$resolved_entrypoint"
          else
            rm -rf "$provider_root"
          fi
        fi
      fi

      if [[ ! -d "$provider_root" ]]; then
        mkdir -p "$provider_root"
        npm_install_bundle "$provider_root" "${package}@${version}"
        entrypoint_path="$provider_root/$entrypoint"
        if [[ ! -f "$entrypoint_path" ]]; then
          entrypoint_path="$(resolve_npm_entrypoint "$provider_root" "$package" "$entrypoint" || true)"
        fi
        if [[ -z "$entrypoint_path" || ! -f "$entrypoint_path" ]]; then
          log "error: npm entrypoint missing for $provider_id: $entrypoint_path"
          exit 5
        fi
        echo "$version" > "$version_marker"
      fi

      command_path="$node_bin"
      entrypoint_rel="${entrypoint_path#"$bundle_dir/"}"
      args_json="$(PROVIDER_ENTRYPOINT="$entrypoint_rel" PROVIDER_ARGS_JSON="$args_json" run_python - <<'PY'
import json
import os

args = [os.environ["PROVIDER_ENTRYPOINT"]]
extra = json.loads(os.environ["PROVIDER_ARGS_JSON"] or "[]")
args.extend(extra)
print(json.dumps(args, separators=(",", ":")))
PY
)"
      ;;
    python)
      if [[ -z "$version" || -z "$package" || -z "$entrypoint" ]]; then
        log "error: missing python metadata for $provider_id"
        exit 5
      fi

      venv_dir="$provider_root/venv"
      entrypoint_path="$(venv_exe "$venv_dir" "$entrypoint")"
      if [[ -f "$version_marker" ]]; then
        if [[ "$(cat "$version_marker" 2>/dev/null || true)" != "$version" || ! -f "$entrypoint_path" ]]; then
          rm -rf "$provider_root"
        fi
      fi

      if [[ ! -d "$provider_root" ]]; then
        mkdir -p "$provider_root"
        "$python_bin" -m venv "$venv_dir"
        venv_python="$(venv_exe "$venv_dir" "python")"
        ensure_venv_pip "$venv_python"

        package_spec="$package"
        if [[ "$package" != http://* && "$package" != https://* ]]; then
          package_spec="${package}==${version}"
        fi

        PIP_DISABLE_PIP_VERSION_CHECK=1 \
        PIP_NO_INPUT=1 \
        "$venv_python" -m pip install --disable-pip-version-check --no-input "$package_spec"

        entrypoint_path="$(venv_exe "$venv_dir" "$entrypoint")"
        if [[ ! -f "$entrypoint_path" ]]; then
          log "error: python entrypoint missing for $provider_id: $entrypoint_path"
          exit 5
        fi
        echo "$version" > "$version_marker"
      fi

      command_path="$entrypoint_path"
      ;;
    *)
      continue
      ;;
  esac

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
  run_python - <<'PY' >> "$providers_out"
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

node_sha="$(sha256_file "$node_bin")"
NODE_VERSION_ENV="$NODE_VERSION" \
NODE_OS_ENV="$os" \
NODE_ARCH_ENV="$arch" \
NODE_SHA_ENV="$node_sha" \
NODE_ROOT_REL_ENV="$node_root_rel" \
NODE_BIN_REL_ENV="$node_bin_rel" \
NODE_NPM_REL_ENV="$npm_cli_rel" \
run_python - <<'PY' >> "$runtimes_out"
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

python_sha="$(sha256_file "$python_bin")"
PYTHON_VERSION_ENV="$PYTHON_VERSION" \
PYTHON_OS_ENV="$os" \
PYTHON_ARCH_ENV="$arch" \
PYTHON_SHA_ENV="$python_sha" \
PYTHON_ROOT_REL_ENV="$python_root_rel" \
PYTHON_BIN_REL_ENV="$python_bin_rel" \
run_python - <<'PY' >> "$runtimes_out"
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

if [[ "${CTX_BUNDLE_PODMAN:-0}" == "1" ]]; then
  if [[ ! -f "$podman_bin" ]]; then
    log "error: podman binary missing at $podman_bin"
    exit 4
  fi
  podman_sha="$(sha256_file "$podman_bin")"
  PODMAN_VERSION_ENV="$PODMAN_VERSION" \
  PODMAN_OS_ENV="$os" \
  PODMAN_ARCH_ENV="$arch" \
  PODMAN_SHA_ENV="$podman_sha" \
  PODMAN_ROOT_REL_ENV="$podman_root_rel" \
  PODMAN_BIN_REL_ENV="$podman_bin_rel" \
  run_python - <<'PY' >> "$runtimes_out"
import json
import os

entry = {
    "id": "podman",
    "version": os.environ["PODMAN_VERSION_ENV"],
    "os": os.environ["PODMAN_OS_ENV"],
    "arch": os.environ["PODMAN_ARCH_ENV"],
    "sha256": os.environ["PODMAN_SHA_ENV"],
    "root": os.environ["PODMAN_ROOT_REL_ENV"],
    "bin": os.environ["PODMAN_BIN_REL_ENV"],
}
print(json.dumps(entry, separators=(",", ":")))
PY
fi

manifest_path="$bundle_dir/manifest.json"
GENERATED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
PROVIDERS_OUT="$providers_out" \
RUNTIMES_OUT="$runtimes_out" \
GENERATED_AT_ENV="$GENERATED_AT" \
MANIFEST_PATH="$manifest_path" \
BUNDLE_APPEND_ENV="${CTX_BUNDLE_APPEND:-}" \
run_python - <<'PY'
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

append = os.environ.get("BUNDLE_APPEND_ENV", "").strip().lower() in ("1", "true", "yes")
if append and manifest_path.exists():
    try:
        existing = json.loads(manifest_path.read_text())
    except Exception:
        existing = None
    if isinstance(existing, dict) and existing.get("version") == manifest["version"]:
        providers_by_key = {}
        for entry in existing.get("providers", []) or []:
            key = (
                entry.get("id"),
                entry.get("protocol"),
                entry.get("os"),
                entry.get("arch"),
            )
            providers_by_key[key] = entry
        for entry in providers:
            key = (
                entry.get("id"),
                entry.get("protocol"),
                entry.get("os"),
                entry.get("arch"),
            )
            providers_by_key[key] = entry
        runtimes_by_key = {}
        for entry in existing.get("runtimes", []) or []:
            key = (entry.get("id"), entry.get("os"), entry.get("arch"))
            runtimes_by_key[key] = entry
        for entry in runtimes:
            key = (entry.get("id"), entry.get("os"), entry.get("arch"))
            runtimes_by_key[key] = entry
        manifest["providers"] = sorted(
            providers_by_key.values(),
            key=lambda e: (
                e.get("id", ""),
                e.get("protocol", ""),
                e.get("os", ""),
                e.get("arch", ""),
            ),
        )
        manifest["runtimes"] = sorted(
            runtimes_by_key.values(),
            key=lambda e: (e.get("id", ""), e.get("os", ""), e.get("arch", "")),
        )

manifest_path.write_text(json.dumps(manifest, indent=2))
PY

rm -f "$providers_src" "$providers_out" "$runtimes_out" "$local_providers_src" || true

echo "$bundle_dir"
