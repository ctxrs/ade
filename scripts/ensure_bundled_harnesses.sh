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

host_os_raw="$(uname -s 2>/dev/null || true)"
host_os="unknown"
case "$host_os_raw" in
  Linux) host_os="linux";;
  Darwin) host_os="macos";;
  MINGW*|MSYS*|CYGWIN*|Windows_NT) host_os="windows";;
esac

host_arch_raw="$(uname -m 2>/dev/null || true)"
host_arch="unknown"
case "$host_arch_raw" in
  x86_64|amd64) host_arch="x86_64";;
  aarch64|arm64) host_arch="aarch64";;
esac

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

run_with_timeout_capture() {
  local timeout_secs="$1"
  shift
  run_python - "$timeout_secs" "$@" <<'PY'
import subprocess
import sys

if len(sys.argv) < 3:
    print("internal error: run_with_timeout_capture requires timeout + command", file=sys.stderr)
    sys.exit(2)

try:
    timeout = float(sys.argv[1])
except ValueError:
    print(f"internal error: invalid timeout {sys.argv[1]!r}", file=sys.stderr)
    sys.exit(2)

cmd = sys.argv[2:]
try:
    result = subprocess.run(
        cmd,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        timeout=timeout,
    )
except subprocess.TimeoutExpired as exc:
    output = exc.stdout or ""
    if output:
        sys.stdout.write(output)
    print(f"timed out after {timeout:.0f}s: {' '.join(cmd)}")
    sys.exit(124)

if result.stdout:
    sys.stdout.write(result.stdout)
sys.exit(result.returncode)
PY
}

maybe_adhoc_codesign_macos_binary() {
  local bin_path="$1"
  if [[ "$os" != "macos" ]]; then
    return 0
  fi
  if [[ -z "$bin_path" || ! -f "$bin_path" ]]; then
    return 0
  fi
  if ! command -v codesign >/dev/null 2>&1; then
    return 0
  fi
  if command -v file >/dev/null 2>&1; then
    local file_desc
    file_desc="$(file -b "$bin_path" 2>/dev/null || true)"
    if [[ "$file_desc" != *"Mach-O"* ]]; then
      return 0
    fi
  fi
  # Re-sign copied local binaries to avoid stale/invalid signatures causing runtime SIGKILL.
  codesign --force --sign - "$bin_path" >/dev/null 2>&1 || true
}

# Build-time container contract:
# - Build-time operations (containerized codex-crp builds + bundled harness image builds)
#   use Docker as the only engine, with Docker buildx required for image bundling.
# - Podman remains a bundled runtime artifact for runtime container execution only.
docker_buildx_output_looks_unhealthy() {
  local output="$1"
  if [[ -z "$output" ]]; then
    return 1
  fi
  if printf '%s\n' "$output" | grep -Eiq 'Cannot load builder|context deadline exceeded'; then
    return 0
  fi
  if printf '%s\n' "$output" | grep -Eiq '(^|[[:space:]])error([[:space:]:]|$)'; then
    return 0
  fi
  return 1
}

docker_probe_health() {
  local quiet="${1:-0}"
  local require_buildx="${2:-1}"
  local info_output=""
  local buildx_output=""

  info_output="$(run_with_timeout_capture "$DOCKER_HEALTH_TIMEOUT_SECS" docker info --format '{{.ServerVersion}}|{{.OSType}}|{{.Architecture}}' 2>&1)" || {
    if [[ "$quiet" != "1" ]]; then
      log "warn: docker info health check failed."
      if [[ -n "$info_output" ]]; then
        log "$info_output"
      fi
    fi
    return 1
  }

  if [[ "$require_buildx" == "1" ]]; then
    buildx_output="$(run_with_timeout_capture "$DOCKER_HEALTH_TIMEOUT_SECS" docker buildx ls 2>&1)" || {
      if [[ "$quiet" != "1" ]]; then
        log "warn: docker buildx ls health check failed."
        if [[ -n "$buildx_output" ]]; then
          log "$buildx_output"
        fi
      fi
      return 1
    }

    if docker_buildx_output_looks_unhealthy "$buildx_output"; then
      if [[ "$quiet" != "1" ]]; then
        log "warn: docker buildx reported unhealthy builders."
        log "$buildx_output"
      fi
      return 1
    fi
  fi

  return 0
}

try_start_docker_desktop_macos() {
  if [[ "$host_os" != "macos" ]]; then
    return 1
  fi
  if ! command -v open >/dev/null 2>&1; then
    return 1
  fi
  if open -gj -a Docker >/dev/null 2>&1 || open -a Docker >/dev/null 2>&1; then
    log "warn: Docker health check failed. Attempting self-heal by launching Docker Desktop and waiting up to ${DOCKER_HEALTH_WAIT_SECS}s."
    return 0
  fi
  return 1
}

ensure_docker_ready_for_builds() {
  local require_buildx="${1:-0}"
  local operation="${2:-build-time container operation}"

  if ! command -v docker >/dev/null 2>&1; then
    log "error: ${operation} requires docker on PATH."
    return 1
  fi

  if [[ "$require_buildx" == "1" ]] && ! docker buildx version >/dev/null 2>&1; then
    log "error: ${operation} requires docker buildx."
    return 1
  fi

  if docker_probe_health "1" "$require_buildx"; then
    return 0
  fi

  if try_start_docker_desktop_macos; then
    local deadline=$((SECONDS + DOCKER_HEALTH_WAIT_SECS))
    while (( SECONDS < deadline )); do
      if docker_probe_health "1" "$require_buildx"; then
        log "docker daemon became healthy after Docker Desktop startup."
        return 0
      fi
      sleep 2
    done
  fi

  docker_probe_health "0" "$require_buildx" || true
  log "error: docker daemon is not healthy. Ensure Docker Desktop is running and retry."
  return 1
}

INSTALLER_RS="$ROOT/core/crates/ctx-http/src/installer.rs"
DEFAULT_MATRIX_JSON="$ROOT/core/crates/ctx-http/src/provider_matrix.json"
CACHED_MATRIX_JSON="${HOME:-}/.ctx/providers/provider_matrix.json"
if [[ -n "${CTX_BUNDLE_MATRIX_JSON:-}" ]]; then
  MATRIX_JSON="${CTX_BUNDLE_MATRIX_JSON}"
elif [[ -f "$CACHED_MATRIX_JSON" ]]; then
  MATRIX_JSON="$CACHED_MATRIX_JSON"
else
  MATRIX_JSON="$DEFAULT_MATRIX_JSON"
fi

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
PODMAN_ARCHIVE_SHA256="${PODMAN_ARCHIVE_SHA256:-}"
PODMAN_BIN_REL="${PODMAN_BIN_REL:-}"
PODMAN_EXTRACT_SUBDIR="${PODMAN_EXTRACT_SUBDIR:-}"
PODMAN_GVPROXY_SHA256="${PODMAN_GVPROXY_SHA256:-}"
PODMAN_VFKIT_SHA256="${PODMAN_VFKIT_SHA256:-}"
DOCKER_HEALTH_TIMEOUT_SECS="${CTX_BUNDLE_DOCKER_HEALTH_TIMEOUT_SECS:-8}"
DOCKER_HEALTH_WAIT_SECS="${CTX_BUNDLE_DOCKER_HEALTH_WAIT_SECS:-60}"

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
bundle_dir="$(cd "$bundle_dir" && pwd)"

# Keep build artifacts out of the desktop app bundle resources by default.
# (The app packages `bundles/`, so anything under it is at risk of shipping.)
bundle_build_dir="${CTX_BUNDLE_BUILD_DIR:-$cache_base/.build}"
mkdir -p "$bundle_build_dir"
bundle_build_dir="$(cd "$bundle_build_dir" && pwd)"

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
CODEX_CRP_WORKSPACE="${CTX_BUNDLE_CODEX_CRP_WORKSPACE:-$ROOT/external-harnesses/codex/codex-rs}"
CODEX_CRP_BUILD_MODE="${CTX_BUNDLE_BUILD_CODEX_CRP:-0}"
CLAUDE_CRP_WORKSPACE="${CTX_BUNDLE_CLAUDE_CRP_WORKSPACE:-$ROOT/external-harnesses/claude-crp}"
LOCAL_ADAPTERS_DIR="${CTX_BUNDLE_ADAPTERS_DIR:-$ROOT/harness-adapters}"
LOCAL_ADAPTER_MODE="${CTX_BUNDLE_LOCAL_ADAPTERS:-on}"
BUILD_LOCAL_ADAPTERS="${CTX_BUNDLE_BUILD_LOCAL_ADAPTERS:-0}"
# The bridge is required for bundles; build it when missing unless explicitly disabled.
BUILD_LOCAL_BRIDGE="${CTX_BUNDLE_BUILD_LOCAL_BRIDGE:-1}"
INCLUDE_BRIDGE="${CTX_BUNDLE_INCLUDE_BRIDGE:-1}"

local_adapter_dir() {
  case "${1:-}" in
    amp) printf '%s' "amp-acp" ;;
    pi) printf '%s' "pi-acp" ;;
    goose) printf '%s' "openhands-acp" ;;
    openhands) printf '%s' "openhands-acp" ;;
    droid) printf '%s' "droid-acp" ;;
    *) printf '%s' "" ;;
  esac
}

local_adapter_bin() {
  case "${1:-}" in
    droid) printf '%s' "droid-acp" ;;
    *) printf '%s' "" ;;
  esac
}

get_matrix_version() {
  local provider_id="$1"
  run_python - "$MATRIX_JSON" "$provider_id" <<'PY'
import json
import sys
import re

path = sys.argv[1]
provider_id = sys.argv[2]
data = json.loads(open(path, "r", encoding="utf-8").read())

def parse_version_loose(raw: str):
    if not raw:
        return None
    trimmed = raw.strip().lstrip("v")
    if not trimmed:
        return None
    if trimmed.count(".") == 1:
        trimmed = f"{trimmed}.0"
    match = re.match(r"([0-9]+(?:\.[0-9]+)*)", trimmed)
    if not match:
        return None
    try:
        return tuple(int(part) for part in match.group(1).split("."))
    except ValueError:
        return None

def select_latest_release(candidates):
    best = None
    best_v = None
    for release in candidates:
        parsed = parse_version_loose(str(release.get("version") or ""))
        if parsed is None:
            continue
        if best_v is None or parsed > best_v:
            best_v = parsed
            best = release
    if best is not None:
        return best
    return candidates[-1] if candidates else None

for provider in data.get("providers", []):
    if provider.get("id") == provider_id:
        mi = provider.get("managed_install") or {}
        version = mi.get("version") or ""
        if version:
            print(version)
            sys.exit(0)
        releases = [r for r in provider.get("releases", []) if r.get("status") == "supported"]
        release = select_latest_release(releases)
        print(release.get("version", "") if release else "")
        sys.exit(0)
print("")
PY
}

package_json_version() {
  local package_json="$1"
  run_python - "$package_json" <<'PY'
import json
import sys

path = sys.argv[1]
try:
    with open(path, "r", encoding="utf-8") as fh:
        data = json.load(fh)
except Exception:
    print("")
    sys.exit(0)
value = data.get("version")
print(value if isinstance(value, str) else "")
PY
}

local_adapter_binary_path() {
  local provider_id="$1"
  local dir
  dir="$(local_adapter_dir "$provider_id")"
  local bin
  bin="$(local_adapter_bin "$provider_id")"
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
  local prefer_native="0"
  if [[ "${CTX_BUNDLE_BRIDGE_FORCE_TARGET:-0}" != "1" ]] && [[ "$host_os" == "$os" ]] && [[ "$host_arch" == "$arch" ]]; then
    prefer_native="1"
  fi

  if [[ -n "${CARGO_TARGET_DIR:-}" ]]; then
    if [[ "$prefer_native" == "1" ]]; then
      printf '%s' "$CARGO_TARGET_DIR/release/${BRIDGE_BIN}${BIN_EXT}"
      return 0
    fi
    if [[ -n "${rust_target:-}" ]]; then
      printf '%s' "$CARGO_TARGET_DIR/$rust_target/release/${BRIDGE_BIN}${BIN_EXT}"
    else
      printf '%s' "$CARGO_TARGET_DIR/release/${BRIDGE_BIN}${BIN_EXT}"
    fi
    return 0
  fi
  if [[ "$prefer_native" == "1" ]]; then
    printf '%s' "$BRIDGE_DIR/target/release/${BRIDGE_BIN}${BIN_EXT}"
    return 0
  fi
  printf '%s' "$BRIDGE_DIR/target/$rust_target/release/${BRIDGE_BIN}${BIN_EXT}"
}

require_bridge_binary() {
  if [[ ! -d "$BRIDGE_DIR" ]]; then
    log "error: missing acp-crp-bridge source dir at $BRIDGE_DIR"
    exit 5
  fi

  build_bridge_in_container() {
    if ! ensure_docker_ready_for_builds "0" "acp-crp-bridge container build"; then
      log "error: building acp-crp-bridge for ${os}/${arch} requires Docker on PATH and a healthy daemon."
      log "       Ensure Docker Desktop (or Docker Engine) is running and retry."
      exit 5
    fi

    local target_dir="${CARGO_TARGET_DIR:-$BRIDGE_DIR/target}"
    mkdir -p "$target_dir"

    local platform="linux/arm64"
    if [[ "$arch" == "x86_64" ]]; then
      platform="linux/amd64"
    fi

    local image="${CTX_BUNDLE_RUST_IMAGE:-rust:1}"
    docker run --rm --platform "$platform" \
      -v "$BRIDGE_DIR:/work:rw" \
      -v "$target_dir:/target:rw" \
      -w /work \
      -e CARGO_TARGET_DIR=/target \
      "$image" \
      bash -c "set -euo pipefail; export PATH=\"/usr/local/cargo/bin:\$PATH\"; rustup target add '$rust_target' >/dev/null 2>&1 || true; cargo build --release --target '$rust_target'"
  }

  local bridge_out
  bridge_out="$(local_bridge_binary_path)"
  if [[ ! -f "$bridge_out" ]]; then
    if is_truthy "$BUILD_LOCAL_BRIDGE" || is_truthy "$BUILD_LOCAL_ADAPTERS"; then
      if [[ "$os" == "linux" && "$host_os" != "linux" ]]; then
        build_bridge_in_container
      else
        require_cmd cargo
        if [[ "${CTX_BUNDLE_BRIDGE_FORCE_TARGET:-0}" != "1" ]] && [[ "$host_os" == "$os" ]] && [[ "$host_arch" == "$arch" ]]; then
          (cd "$BRIDGE_DIR" && cargo build --release)
        else
          (cd "$BRIDGE_DIR" && cargo build --release --target "$rust_target")
        fi
      fi
    fi
  fi
  if [[ ! -f "$bridge_out" ]]; then
    log "error: missing acp-crp-bridge binary at $bridge_out"
    exit 5
  fi
}

verify_sha256_if_expected() {
  local path="$1"
  local expected="$2"
  local label="$3"
  if [[ -z "$expected" ]]; then
    return
  fi
  local actual
  actual="$(sha256_file "$path")"
  if [[ "$actual" != "$expected" ]]; then
    log "error: ${label} sha256 mismatch (expected $expected, got $actual)"
    exit 4
  fi
}

local_adapter_amp_entrypoint() {
  local dir
  dir="$(local_adapter_dir "amp")"
  printf '%s' "$LOCAL_ADAPTERS_DIR/$dir/dist/bin/amp-acp.js"
}

local_adapter_node_entrypoint() {
  case "${1:-}" in
    amp)
      local_adapter_amp_entrypoint
      ;;
    pi)
      local dir
      dir="$(local_adapter_dir "pi")"
      printf '%s' "$LOCAL_ADAPTERS_DIR/$dir/dist/bin/pi-acp.js"
      ;;
    goose)
      local dir
      dir="$(local_adapter_dir "goose")"
      printf '%s' "$LOCAL_ADAPTERS_DIR/$dir/dist/bin/goose-acp.js"
      ;;
    openhands)
      local dir
      dir="$(local_adapter_dir "openhands")"
      printf '%s' "$LOCAL_ADAPTERS_DIR/$dir/dist/bin/openhands-acp.js"
      ;;
    *)
      printf '%s' ""
      ;;
  esac
}

local_adapter_root_path() {
  local provider_id="$1"
  local dir
  dir="$(local_adapter_dir "$provider_id")"
  if [[ -z "$dir" ]]; then
    return 1
  fi
  printf '%s' "$LOCAL_ADAPTERS_DIR/$dir"
}

copy_local_node_adapter_payload() {
  local adapter_root="$1"
  local dest_root="$2"
  run_python - "$adapter_root" "$dest_root" <<'PY'
import os
import shutil
import sys

src = sys.argv[1]
dst = sys.argv[2]

if os.path.exists(dst):
    shutil.rmtree(dst)

ignored = {
    ".git",
    ".github",
    "node_modules",
    "target",
    ".next",
    ".turbo",
    "coverage",
}

def ignore(_dir, names):
    return [name for name in names if name in ignored]

shutil.copytree(src, dst, ignore=ignore, symlinks=True)
PY
}

copy_dmg_payload_without_external_symlinks() {
  local mount_root="$1"
  local dest_root="$2"
  run_python - "$mount_root" "$dest_root" <<'PY'
import os
import shutil
import sys
from pathlib import Path

src_root = Path(sys.argv[1]).resolve()
dst_root = Path(sys.argv[2])


def is_within_root(path: Path) -> bool:
    try:
        path.relative_to(src_root)
        return True
    except ValueError:
        return False


def remove_existing(path: Path) -> None:
    try:
        if path.is_symlink() or path.is_file():
            path.unlink()
        elif path.is_dir():
            shutil.rmtree(path)
    except FileNotFoundError:
        return


def copy_tree(src_dir: Path, dst_dir: Path) -> None:
    dst_dir.mkdir(parents=True, exist_ok=True)
    for src_path in src_dir.iterdir():
        dst_path = dst_dir / src_path.name
        if src_path.is_symlink():
            # Some DMGs include links like Applications -> /Applications.
            # Copying those into the bundle tree causes host traversal and permission failures.
            target = os.readlink(src_path)
            resolved = (src_path.parent / target).resolve(strict=False)
            if not is_within_root(resolved):
                continue
            remove_existing(dst_path)
            os.symlink(target, dst_path)
            continue

        if src_path.is_dir():
            if dst_path.is_symlink() or dst_path.is_file():
                remove_existing(dst_path)
            copy_tree(src_path, dst_path)
            continue

        remove_existing(dst_path)
        shutil.copy2(src_path, dst_path)


dst_root.mkdir(parents=True, exist_ok=True)
copy_tree(src_root, dst_root)
PY
}

anthropic_ripgrep_target() {
  case "${os}/${arch}" in
    linux/x86_64) printf '%s' "x64-linux" ;;
    linux/aarch64) printf '%s' "arm64-linux" ;;
    macos/x86_64) printf '%s' "x64-darwin" ;;
    macos/aarch64) printf '%s' "arm64-darwin" ;;
    windows/x86_64) printf '%s' "x64-win32" ;;
    windows/aarch64) printf '%s' "arm64-win32" ;;
    *) printf '%s' "" ;;
  esac
}

prune_anthropic_ripgrep_vendor_root() {
  local vendor_root="$1"
  local label="$2"
  if [[ ! -d "$vendor_root" ]]; then
    return 0
  fi

  local keep_target
  keep_target="$(anthropic_ripgrep_target)"
  if [[ -z "$keep_target" ]]; then
    log "error: unsupported ripgrep prune target for ${os}/${arch}"
    exit 5
  fi
  if [[ ! -d "$vendor_root/$keep_target" ]]; then
    log "error: ${label} ripgrep vendor missing expected target '$keep_target' at $vendor_root"
    exit 5
  fi

  local child
  for child in "$vendor_root"/*; do
    if [[ ! -d "$child" ]]; then
      continue
    fi
    if [[ "$(basename "$child")" == "$keep_target" ]]; then
      continue
    fi
    rm -rf "$child"
  done
}

prune_anthropic_ripgrep_vendors() {
  local provider_root="$1"
  prune_anthropic_ripgrep_vendor_root \
    "$provider_root/node_modules/@anthropic-ai/claude-agent-sdk/vendor/ripgrep" \
    "claude-agent-sdk"
  prune_anthropic_ripgrep_vendor_root \
    "$provider_root/node_modules/@anthropic-ai/claude-code/vendor/ripgrep" \
    "claude-code"
}

prune_napi_keyring_musl_packages() {
  local provider_root="$1"
  if [[ "$os" != "linux" ]]; then
    return 0
  fi
  local napi_root="$provider_root/node_modules/@napi-rs"
  if [[ ! -d "$napi_root" ]]; then
    return 0
  fi

  local pkg
  for pkg in "$napi_root"/keyring-linux-*-musl; do
    if [[ ! -d "$pkg" ]]; then
      continue
    fi
    rm -rf "$pkg"
  done
}

prune_provider_node_payload() {
  local provider_id="$1"
  local provider_root="$2"
  if [[ "$provider_id" == "claude-crp" || "$provider_id" == "claude-cli" ]]; then
    prune_anthropic_ripgrep_vendors "$provider_root"
  fi
  prune_napi_keyring_musl_packages "$provider_root"
}

write_cline_bundle_stubs() {
  local provider_root="$1"
  local node_modules_root="$provider_root/node_modules"
  mkdir -p "$node_modules_root/vscode" "$node_modules_root/grpc-health-check"
  cat > "$node_modules_root/vscode/index.js" <<'JS'
"use strict";

const noop = () => {};
const disposable = { dispose: noop };

module.exports = {
  workspace: {
    getConfiguration: () => ({ get: () => undefined }),
    workspaceFolders: [],
  },
  window: {
    showInformationMessage: async () => undefined,
    showWarningMessage: async () => undefined,
    showErrorMessage: async () => undefined,
    showInputBox: async () => undefined,
    showOpenDialog: async () => undefined,
    showSaveDialog: async () => undefined,
    createOutputChannel: () => ({ appendLine: noop, append: noop, clear: noop, dispose: noop }),
  },
  commands: {
    executeCommand: async () => undefined,
    registerCommand: () => disposable,
  },
  env: {
    clipboard: {
      readText: async () => "",
      writeText: async () => undefined,
    },
  },
  EventEmitter: class EventEmitter {
    constructor() {
      this.listeners = [];
      this.event = (listener) => {
        this.listeners.push(listener);
        return { dispose: () => { this.listeners = this.listeners.filter((entry) => entry !== listener); } };
      };
    }
    fire(value) {
      for (const listener of this.listeners) listener(value);
    }
    dispose() {
      this.listeners = [];
    }
  },
  ExtensionMode: { Development: 0, Production: 1, Test: 2 },
  ExtensionKind: { UI: 1, Workspace: 2 },
};
JS

  cat > "$node_modules_root/grpc-health-check/index.js" <<'JS'
"use strict";

const path = require("node:path");

module.exports = {
  protoPath: path.join(__dirname, "health.proto"),
};
JS

  cat > "$node_modules_root/grpc-health-check/health.proto" <<'PROTO'
syntax = "proto3";

package grpc.health.v1;

message HealthCheckRequest {
  string service = 1;
}

message HealthCheckResponse {
  enum ServingStatus {
    UNKNOWN = 0;
    SERVING = 1;
    NOT_SERVING = 2;
    SERVICE_UNKNOWN = 3;
  }
  ServingStatus status = 1;
}

service Health {
  rpc Check(HealthCheckRequest) returns (HealthCheckResponse);
}
PROTO
}

patch_cline_standalone_runtime() {
  local provider_root="$1"
  local cline_js="$provider_root/dist-standalone/cline-acp.js"
  if [[ ! -f "$cline_js" ]]; then
    log "error: cline standalone runtime missing: $cline_js"
    exit 5
  fi

  run_python - "$cline_js" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
text = path.read_text(encoding="utf-8")

original_manager = """var ExternalHostBridgeClientManager = class {
  workspaceClient;
  envClient;
  windowClient;
  diffClient;
  constructor() {
    const address = process.env.HOST_BRIDGE_ADDRESS || `localhost:${HOSTBRIDGE_PORT}`;
    this.workspaceClient = new WorkspaceServiceClientImpl(address);
    this.envClient = new EnvServiceClientImpl(address);
    this.windowClient = new WindowServiceClientImpl(address);
    this.diffClient = new DiffServiceClientImpl(address);
  }
};"""

patched_manager = """var ExternalHostBridgeClientManager = class {
  workspaceClient;
  envClient;
  windowClient;
  diffClient;
  constructor() {
    const useExternalHostBridge = process.env.CTX_CLINE_EXTERNAL_HOSTBRIDGE === \"1\" || !!process.env.HOST_BRIDGE_ADDRESS;
    if (!useExternalHostBridge) {
      let clipboardText = \"\";
      let activeDiffId = \"\";
      const fallbackWorkspace = process.env.CTX_CLINE_WORKSPACE_CWD || process.env.PWD || process.cwd();
      this.workspaceClient = {
        getWorkspacePaths: async () => ({ paths: [fallbackWorkspace] }),
        saveOpenDocumentIfDirty: async () => ({ saved: false }),
        getDiagnostics: async () => ({ fileDiagnostics: [] }),
        openProblemsPanel: async () => ({}),
        openInFileExplorerPanel: async () => ({}),
        openClineSidebarPanel: async () => ({}),
        openTerminalPanel: async () => ({}),
        executeCommandInTerminal: async () => ({ success: true })
      };
      this.envClient = {
        clipboardWriteText: async (request5) => {
          clipboardText = request5?.value ?? \"\";
          return {};
        },
        clipboardReadText: async () => ({ value: clipboardText }),
        getHostVersion: async () => ({ platform: \"standalone\", version: \"0.0.0\", clineType: \"cli\" }),
        getIdeRedirectUri: async () => ({ value: \"\" }),
        getTelemetrySettings: async () => ({ isEnabled: 0 }),
        subscribeToTelemetrySettings: (_request5, callbacks) => {
          callbacks?.onResponse?.({ isEnabled: 0 });
          return () => {
          };
        },
        shutdown: async () => ({})
      };
      this.windowClient = {
        showTextDocument: async () => ({}),
        showOpenDialogue: async () => ({ paths: [] }),
        showMessage: async () => ({ selectedOption: \"\" }),
        showInputBox: async () => ({ value: \"\" }),
        showSaveDialog: async () => ({ path: \"\" }),
        openFile: async () => ({}),
        openSettings: async () => ({}),
        getOpenTabs: async () => ({ paths: [] }),
        getVisibleTabs: async () => ({ paths: [] }),
        getActiveEditor: async () => ({ path: \"\" })
      };
      this.diffClient = {
        openDiff: async () => {
          activeDiffId = activeDiffId || \"ctx-cline-diff\";
          return { diffId: activeDiffId };
        },
        getDocumentText: async () => ({ content: \"\" }),
        replaceText: async () => ({}),
        scrollDiff: async () => ({}),
        truncateDocument: async () => ({}),
        saveDocument: async () => ({}),
        closeAllDiffs: async () => ({}),
        openMultiFileDiff: async () => ({})
      };
      return;
    }
    const address = process.env.HOST_BRIDGE_ADDRESS || `localhost:${HOSTBRIDGE_PORT}`;
    this.workspaceClient = new WorkspaceServiceClientImpl(address);
    this.envClient = new EnvServiceClientImpl(address);
    this.windowClient = new WindowServiceClientImpl(address);
    this.diffClient = new DiffServiceClientImpl(address);
  }
};"""

if "const useExternalHostBridge = process.env.CTX_CLINE_EXTERNAL_HOSTBRIDGE" not in text:
    if original_manager not in text:
        raise SystemExit("failed to patch cline host bridge manager block")
    text = text.replace(original_manager, patched_manager, 1)

fallback_workspace_decl = "      const fallbackWorkspace = process.env.CTX_CLINE_WORKSPACE_CWD || process.env.PWD || process.cwd();"
broken_fallback_workspace_decl = "      let activeDiffId = \"\";\\n      const fallbackWorkspace = process.env.CTX_CLINE_WORKSPACE_CWD || process.env.PWD || process.cwd();"
if broken_fallback_workspace_decl in text:
    text = text.replace(
        broken_fallback_workspace_decl,
        "      let activeDiffId = \"\";\\n" + fallback_workspace_decl,
        1,
    )
if fallback_workspace_decl not in text:
    marker = "      let activeDiffId = \"\";"
    if marker not in text:
        raise SystemExit("failed to patch cline fallback workspace declaration")
    text = text.replace(marker, marker + "\n" + fallback_workspace_decl, 1)

workspace_paths_line = "        getWorkspacePaths: async () => ({ paths: [process.cwd()] }),"
workspace_paths_fallback = "        getWorkspacePaths: async () => ({ paths: [fallbackWorkspace] }),"
if workspace_paths_fallback not in text:
    if workspace_paths_line not in text:
        raise SystemExit("failed to patch cline fallback workspace path")
    text = text.replace(workspace_paths_line, workspace_paths_fallback, 1)

original_wait = "    await waitForHostBridgeReady();"
patched_wait = """    const useExternalHostBridge = process.env.CTX_CLINE_EXTERNAL_HOSTBRIDGE === \"1\" || !!process.env.HOST_BRIDGE_ADDRESS;
    if (useExternalHostBridge) {
      await waitForHostBridgeReady();
    }"""
if patched_wait not in text:
    if original_wait not in text:
        raise SystemExit("failed to patch cline host bridge wait block")
    text = text.replace(original_wait, patched_wait, 1)

stdout_log_line = "  console.log(`[${timestamp}]`, \"#bot.cline.server.ts\", ...args3);"
stderr_log_line = "  console.error(`[${timestamp}]`, \"#bot.cline.server.ts\", ...args3);"
if stderr_log_line not in text:
    if stdout_log_line not in text:
        raise SystemExit("failed to patch cline standalone log stream")
    text = text.replace(stdout_log_line, stderr_log_line, 1)

extension_dir_line = "  const EXTENSION_DIR = import_path91.default.join(INSTALL_DIR, \"extension\");"
extension_dir_block = """  const extensionPackagePath = import_path91.default.join(INSTALL_DIR, \"extension\", \"package.json\");
  const EXTENSION_DIR = fs49.existsSync(extensionPackagePath) ? import_path91.default.join(INSTALL_DIR, \"extension\") : INSTALL_DIR;"""
if extension_dir_block not in text:
    if extension_dir_line not in text:
        raise SystemExit("failed to patch cline extension directory selection")
    text = text.replace(extension_dir_line, extension_dir_block, 1)

extension_package_read = "    packageJSON: readJson(import_path91.default.join(EXTENSION_DIR, \"package.json\")),"
extension_package_fallback = "    packageJSON: fs49.existsSync(import_path91.default.join(EXTENSION_DIR, \"package.json\")) ? readJson(import_path91.default.join(EXTENSION_DIR, \"package.json\")) : package_default,"
if extension_package_fallback not in text:
    if extension_package_read not in text:
        raise SystemExit("failed to patch cline extension package metadata fallback")
    text = text.replace(extension_package_read, extension_package_fallback, 1)

runtime_chdir_line = "    process.chdir(path71.dirname((0, import_node_url6.fileURLToPath)(_importMetaUrl)));"
runtime_chdir_block = """    process.chdir(path71.dirname((0, import_node_url6.fileURLToPath)(_importMetaUrl)));
    if (cwd && typeof cwd === \"string\" && cwd.length > 0) {
      process.env.CTX_CLINE_WORKSPACE_CWD = cwd;
    }"""
if runtime_chdir_block not in text:
    if runtime_chdir_line not in text:
        raise SystemExit("failed to patch cline runtime workspace cwd propagation")
    text = text.replace(runtime_chdir_line, runtime_chdir_block, 1)

broken_secret_destructure_line = "    openAiApiKey: resolvedOpenAiApiKey,"
if broken_secret_destructure_line in text:
    text = text.replace(broken_secret_destructure_line, "    openAiApiKey,", 1)

secrets_env_override_lines = """  const endpointOpenAiApiKey = (process.env.OPENAI_API_KEY || "").trim();
  const resolvedOpenAiApiKey = endpointOpenAiApiKey.length > 0 ? endpointOpenAiApiKey : openAiApiKey;
"""
if secrets_env_override_lines in text:
    text = text.replace(secrets_env_override_lines, "", 1)

secret_store_get_line = """  get(key) {
    return Promise.resolve(this.data.get(key));
  }"""
old_secret_store_get_override = """  get(key) {
    if (key === "openAiApiKey") {
      const endpointOpenAiApiKey = (process.env.OPENAI_API_KEY || "").trim();
      if (endpointOpenAiApiKey.length > 0) {
        return Promise.resolve(endpointOpenAiApiKey);
      }
    }
    return Promise.resolve(this.data.get(key));
  }"""
secret_store_get_override = """  get(key) {
    if (key === "openAiApiKey" || key === "openRouterApiKey") {
      const endpointOpenAiApiKey = (process.env.OPENAI_API_KEY || "").trim();
      if (endpointOpenAiApiKey.length > 0) {
        return Promise.resolve(endpointOpenAiApiKey);
      }
    }
    return Promise.resolve(this.data.get(key));
  }"""
if "if (key === \"openAiApiKey\" || key === \"openRouterApiKey\") {" not in text:
    if old_secret_store_get_override in text:
        text = text.replace(old_secret_store_get_override, secret_store_get_override, 1)
    elif secret_store_get_line in text:
        text = text.replace(secret_store_get_line, secret_store_get_override, 1)
    else:
        raise SystemExit("failed to patch cline secret store openai env projection")

provider_default_block = """    let apiProvider;
    if (planModeApiProvider) {
      apiProvider = planModeApiProvider;
    } else {
      apiProvider = "openrouter";
    }"""
provider_default_override = """    const endpointOpenAiApiKey = (process.env.OPENAI_API_KEY || "").trim();
    const endpointOpenAiBaseUrl = (process.env.OPENAI_BASE_URL || "").trim();
    const endpointOpenAiModel = (process.env.OPENAI_MODEL || "").trim();
    const endpointHasApiConfig = endpointOpenAiApiKey.length > 0 && endpointOpenAiBaseUrl.length > 0;
    const endpointBaseHostname = (() => {
      if (!endpointHasApiConfig) {
        return "";
      }
      try {
        return new URL(endpointOpenAiBaseUrl).hostname.toLowerCase();
      } catch (_error) {
        return "";
      }
    })();
    const endpointUsesOpenRouter = endpointBaseHostname === "openrouter.ai" || endpointBaseHostname.endsWith(".openrouter.ai");
    let apiProvider;
    if (endpointHasApiConfig) {
      apiProvider = endpointUsesOpenRouter ? "openrouter" : "openai";
    } else if (planModeApiProvider) {
      apiProvider = planModeApiProvider;
    } else {
      apiProvider = "openrouter";
    }"""
old_provider_default_override = """    const endpointOpenAiApiKey = (process.env.OPENAI_API_KEY || "").trim();
    const endpointOpenAiBaseUrl = (process.env.OPENAI_BASE_URL || "").trim();
    const endpointOpenAiModel = (process.env.OPENAI_MODEL || "").trim();
    const endpointHasOpenAiConfig = endpointOpenAiApiKey.length > 0 && endpointOpenAiBaseUrl.length > 0;
    let apiProvider;
    if (endpointHasOpenAiConfig) {
      apiProvider = "openai";
    } else if (planModeApiProvider) {
      apiProvider = planModeApiProvider;
    } else {
      apiProvider = "openrouter";
    }"""
if "const endpointHasApiConfig = endpointOpenAiApiKey.length > 0 && endpointOpenAiBaseUrl.length > 0;" not in text:
    if old_provider_default_override in text:
        text = text.replace(old_provider_default_override, provider_default_override, 1)
    elif provider_default_block in text:
        text = text.replace(provider_default_block, provider_default_override, 1)
    else:
        raise SystemExit("failed to patch cline provider default selection")

if "openAiBaseUrl: endpointHasApiConfig && !endpointUsesOpenRouter ? endpointOpenAiBaseUrl : openAiBaseUrl," not in text:
    if "      openAiBaseUrl: endpointHasOpenAiConfig ? endpointOpenAiBaseUrl : openAiBaseUrl,\n" in text:
        text = text.replace(
            "      openAiBaseUrl: endpointHasOpenAiConfig ? endpointOpenAiBaseUrl : openAiBaseUrl,\n",
            "      openAiBaseUrl: endpointHasApiConfig && !endpointUsesOpenRouter ? endpointOpenAiBaseUrl : openAiBaseUrl,\n",
            1,
        )
    elif "      openAiBaseUrl,\n" in text:
        text = text.replace(
            "      openAiBaseUrl,\n",
            "      openAiBaseUrl: endpointHasApiConfig && !endpointUsesOpenRouter ? endpointOpenAiBaseUrl : openAiBaseUrl,\n",
            1,
        )
    else:
        raise SystemExit("failed to patch cline openai base url override")

if "planModeApiProvider: endpointHasApiConfig ? (endpointUsesOpenRouter ? \"openrouter\" : \"openai\") : planModeApiProvider || apiProvider," not in text:
    if "      planModeApiProvider: endpointHasOpenAiConfig ? \"openai\" : planModeApiProvider || apiProvider,\n" in text:
        text = text.replace(
            "      planModeApiProvider: endpointHasOpenAiConfig ? \"openai\" : planModeApiProvider || apiProvider,\n",
            "      planModeApiProvider: endpointHasApiConfig ? (endpointUsesOpenRouter ? \"openrouter\" : \"openai\") : planModeApiProvider || apiProvider,\n",
            1,
        )
    elif "      planModeApiProvider: planModeApiProvider || apiProvider,\n" in text:
        text = text.replace(
            "      planModeApiProvider: planModeApiProvider || apiProvider,\n",
            "      planModeApiProvider: endpointHasApiConfig ? (endpointUsesOpenRouter ? \"openrouter\" : \"openai\") : planModeApiProvider || apiProvider,\n",
            1,
        )
    else:
        raise SystemExit("failed to patch cline plan provider override")

if "planModeOpenAiModelId: endpointHasApiConfig && !endpointUsesOpenRouter && endpointOpenAiModel.length > 0 ? endpointOpenAiModel : planModeOpenAiModelId," not in text:
    if "      planModeOpenAiModelId: endpointHasOpenAiConfig && endpointOpenAiModel.length > 0 ? endpointOpenAiModel : planModeOpenAiModelId,\n" in text:
        text = text.replace(
            "      planModeOpenAiModelId: endpointHasOpenAiConfig && endpointOpenAiModel.length > 0 ? endpointOpenAiModel : planModeOpenAiModelId,\n",
            "      planModeOpenAiModelId: endpointHasApiConfig && !endpointUsesOpenRouter && endpointOpenAiModel.length > 0 ? endpointOpenAiModel : planModeOpenAiModelId,\n",
            1,
        )
    elif "      planModeOpenAiModelId,\n" in text:
        text = text.replace(
            "      planModeOpenAiModelId,\n",
            "      planModeOpenAiModelId: endpointHasApiConfig && !endpointUsesOpenRouter && endpointOpenAiModel.length > 0 ? endpointOpenAiModel : planModeOpenAiModelId,\n",
            1,
        )
    else:
        raise SystemExit("failed to patch cline plan model override")

if "planModeOpenRouterModelId: endpointHasApiConfig && endpointUsesOpenRouter && endpointOpenAiModel.length > 0 ? endpointOpenAiModel : planModeOpenRouterModelId," not in text:
    if "      planModeOpenRouterModelId,\n" not in text:
        raise SystemExit("failed to patch cline plan openrouter model override")
    text = text.replace(
        "      planModeOpenRouterModelId,\n",
        "      planModeOpenRouterModelId: endpointHasApiConfig && endpointUsesOpenRouter && endpointOpenAiModel.length > 0 ? endpointOpenAiModel : planModeOpenRouterModelId,\n",
        1,
    )

if "actModeApiProvider: endpointHasApiConfig ? (endpointUsesOpenRouter ? \"openrouter\" : \"openai\") : actModeApiProvider || apiProvider," not in text:
    if "      actModeApiProvider: endpointHasOpenAiConfig ? \"openai\" : actModeApiProvider || apiProvider,\n" in text:
        text = text.replace(
            "      actModeApiProvider: endpointHasOpenAiConfig ? \"openai\" : actModeApiProvider || apiProvider,\n",
            "      actModeApiProvider: endpointHasApiConfig ? (endpointUsesOpenRouter ? \"openrouter\" : \"openai\") : actModeApiProvider || apiProvider,\n",
            1,
        )
    elif "      actModeApiProvider: actModeApiProvider || apiProvider,\n" in text:
        text = text.replace(
            "      actModeApiProvider: actModeApiProvider || apiProvider,\n",
            "      actModeApiProvider: endpointHasApiConfig ? (endpointUsesOpenRouter ? \"openrouter\" : \"openai\") : actModeApiProvider || apiProvider,\n",
            1,
        )
    else:
        raise SystemExit("failed to patch cline act provider override")

if "actModeOpenAiModelId: endpointHasApiConfig && !endpointUsesOpenRouter && endpointOpenAiModel.length > 0 ? endpointOpenAiModel : actModeOpenAiModelId," not in text:
    if "      actModeOpenAiModelId: endpointHasOpenAiConfig && endpointOpenAiModel.length > 0 ? endpointOpenAiModel : actModeOpenAiModelId,\n" in text:
        text = text.replace(
            "      actModeOpenAiModelId: endpointHasOpenAiConfig && endpointOpenAiModel.length > 0 ? endpointOpenAiModel : actModeOpenAiModelId,\n",
            "      actModeOpenAiModelId: endpointHasApiConfig && !endpointUsesOpenRouter && endpointOpenAiModel.length > 0 ? endpointOpenAiModel : actModeOpenAiModelId,\n",
            1,
        )
    elif "      actModeOpenAiModelId,\n" in text:
        text = text.replace(
            "      actModeOpenAiModelId,\n",
            "      actModeOpenAiModelId: endpointHasApiConfig && !endpointUsesOpenRouter && endpointOpenAiModel.length > 0 ? endpointOpenAiModel : actModeOpenAiModelId,\n",
            1,
        )
    else:
        raise SystemExit("failed to patch cline act model override")

if "actModeOpenRouterModelId: endpointHasApiConfig && endpointUsesOpenRouter && endpointOpenAiModel.length > 0 ? endpointOpenAiModel : actModeOpenRouterModelId," not in text:
    if "      actModeOpenRouterModelId,\n" not in text:
        raise SystemExit("failed to patch cline act openrouter model override")
    text = text.replace(
        "      actModeOpenRouterModelId,\n",
        "      actModeOpenRouterModelId: endpointHasApiConfig && endpointUsesOpenRouter && endpointOpenAiModel.length > 0 ? endpointOpenAiModel : actModeOpenRouterModelId,\n",
        1,
    )

path.write_text(text, encoding="utf-8")
PY
}

build_local_adapters() {
  if [[ ! -d "$LOCAL_ADAPTERS_DIR" ]]; then
    log "error: local adapters dir missing: $LOCAL_ADAPTERS_DIR"
    exit 4
  fi

  local node_adapter_id
  for node_adapter_id in amp pi goose openhands; do
    if ! provider_selected_for_bundle "$node_adapter_id"; then
      continue
    fi
    local entrypoint
    entrypoint="$(local_adapter_node_entrypoint "$node_adapter_id")"
    if [[ -z "$entrypoint" || -f "$entrypoint" ]]; then
      continue
    fi
    local adapter_dir
    adapter_dir="$(local_adapter_dir "$node_adapter_id")"
    if [[ -d "$LOCAL_ADAPTERS_DIR/$adapter_dir" ]]; then
      if command -v pnpm >/dev/null 2>&1; then
        (cd "$LOCAL_ADAPTERS_DIR/$adapter_dir" && pnpm install --ignore-scripts)
        (cd "$LOCAL_ADAPTERS_DIR/$adapter_dir" && pnpm run build)
      else
        require_cmd npm
        (cd "$LOCAL_ADAPTERS_DIR/$adapter_dir" && npm install --ignore-scripts)
        (cd "$LOCAL_ADAPTERS_DIR/$adapter_dir" && npm run build)
      fi
    fi
  done

  local id
  for id in droid; do
    local dir
    dir="$(local_adapter_dir "$id")"
    local bin
    bin="$(local_adapter_bin "$id")"
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

ensure_podman_macos_helpers() {
  local podman_root_abs="$1"
  if [[ "$os" != "macos" ]]; then
    return
  fi
  # Podman machine on macOS requires helper binaries (gvproxy, vfkit) that must be colocated
  # under $BINDIR/../libexec/podman for out-of-the-box behavior.
  local helpers_dir="$podman_root_abs/usr/libexec/podman"
  mkdir -p "$helpers_dir"

  local gvproxy_path="$helpers_dir/gvproxy"
  local vfkit_path="$helpers_dir/vfkit"

  local gvproxy_url="${PODMAN_GVPROXY_URL:-https://github.com/containers/gvisor-tap-vsock/releases/download/v0.8.8/gvproxy-darwin}"
  local vfkit_url="${PODMAN_VFKIT_URL:-https://github.com/crc-org/vfkit/releases/download/v0.6.3/vfkit-unsigned}"

  if [[ ! -f "$gvproxy_path" ]]; then
    local tmp
    tmp="$(mktemp -p "$helpers_dir" "gvproxy.XXXXXX")"
    fetch_file "$gvproxy_url" "$tmp"
    verify_sha256_if_expected "$tmp" "$PODMAN_GVPROXY_SHA256" "podman helper gvproxy"
    mv "$tmp" "$gvproxy_path"
  else
    verify_sha256_if_expected "$gvproxy_path" "$PODMAN_GVPROXY_SHA256" "podman helper gvproxy"
  fi
  if [[ ! -f "$vfkit_path" ]]; then
    local tmp
    tmp="$(mktemp -p "$helpers_dir" "vfkit.XXXXXX")"
    fetch_file "$vfkit_url" "$tmp"
    verify_sha256_if_expected "$tmp" "$PODMAN_VFKIT_SHA256" "podman helper vfkit"
    mv "$tmp" "$vfkit_path"
  else
    verify_sha256_if_expected "$vfkit_path" "$PODMAN_VFKIT_SHA256" "podman helper vfkit"
  fi

  chmod +x "$gvproxy_path" "$vfkit_path" || true
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
    ensure_podman_macos_helpers "$podman_root_abs"
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
  verify_sha256_if_expected "$archive_path" "$PODMAN_ARCHIVE_SHA256" "podman archive"

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

  ensure_podman_macos_helpers "$podman_root_abs"
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
  local package_spec="${2:-}"
  local install_mode="${3:-package}"
  local cache_dir="$install_dir/.npm-cache"
  mkdir -p "$cache_dir"

  local npm_node_bin="$node_bin"
  local npm_cli_bin="$npm_cli"
  local use_system_npm="0"
  local ignore_scripts="true"

  if [[ "$os" != "$host_os" ]]; then
    ignore_scripts="true"

    # Cross-target npm providers are JS-only assets; install them using a host-compatible
    # runtime while writing files into the target provider root.
    local host_node_target=""
    case "${host_os}/${host_arch}" in
      linux/x86_64) host_node_target="linux-x64" ;;
      linux/aarch64) host_node_target="linux-arm64" ;;
      macos/x86_64) host_node_target="darwin-x64" ;;
      macos/aarch64) host_node_target="darwin-arm64" ;;
      windows/x86_64) host_node_target="win-x64" ;;
      windows/aarch64) host_node_target="win-arm64" ;;
    esac

    if [[ -n "$host_node_target" ]]; then
      local host_node_root_rel="runtimes/node/${host_os}/${host_arch}/node-v${NODE_VERSION}-${host_node_target}"
      local host_node_root="$bundle_dir/$host_node_root_rel"
      local host_node_bin_rel="bin/node"
      local host_npm_cli_rel="lib/node_modules/npm/bin/npm-cli.js"
      if [[ "$host_os" == "windows" ]]; then
        host_node_bin_rel="node.exe"
        host_npm_cli_rel="node_modules/npm/bin/npm-cli.js"
      fi
      local bundled_host_node_bin="$host_node_root/$host_node_bin_rel"
      local bundled_host_npm_cli="$host_node_root/$host_npm_cli_rel"
      if [[ -f "$bundled_host_node_bin" && -f "$bundled_host_npm_cli" ]]; then
        npm_node_bin="$bundled_host_node_bin"
        npm_cli_bin="$bundled_host_npm_cli"
      else
        use_system_npm="1"
      fi
    else
      use_system_npm="1"
    fi
  fi

  if [[ "$use_system_npm" == "1" ]]; then
    if command -v pnpm >/dev/null 2>&1; then
      if [[ "$install_mode" == "project" ]]; then
        npm_config_ignore_scripts="$ignore_scripts" \
        pnpm --dir "$install_dir" install --prod --ignore-scripts --reporter=silent
      else
        npm_config_ignore_scripts="$ignore_scripts" \
        pnpm --dir "$install_dir" add --ignore-scripts --lockfile=false --reporter=silent "$package_spec"
      fi
    else
      require_cmd npm
      if [[ "$install_mode" == "project" ]]; then
        npm_config_update_notifier="false" \
        npm_config_fund="false" \
        npm_config_audit="false" \
        npm_config_progress="false" \
        npm_config_cache="$cache_dir" \
        npm_config_ignore_scripts="$ignore_scripts" \
        npm install --prefix "$install_dir" --omit=dev --no-audit --no-fund --silent --ignore-scripts
      else
        npm_config_update_notifier="false" \
        npm_config_fund="false" \
        npm_config_audit="false" \
        npm_config_progress="false" \
        npm_config_cache="$cache_dir" \
        npm_config_ignore_scripts="$ignore_scripts" \
        npm install --prefix "$install_dir" --no-audit --no-fund --silent --ignore-scripts "$package_spec"
      fi
    fi
    rm -rf "$cache_dir" || true
    return
  fi

  local node_bin_dir
  node_bin_dir="$(dirname "$npm_node_bin")"
  local path_sep=":"
  if [[ "$os" == "windows" ]]; then
    path_sep=";"
  fi
  local install_args=(
    install
    --prefix "$install_dir"
    --no-audit
    --no-fund
    --silent
    --ignore-scripts
  )
  if [[ "$install_mode" == "project" ]]; then
    install_args+=(--omit=dev)
  else
    install_args+=("$package_spec")
  fi

  PATH="${node_bin_dir}${path_sep}${PATH:-}" \
  npm_config_update_notifier="false" \
  npm_config_fund="false" \
  npm_config_audit="false" \
  npm_config_progress="false" \
  npm_config_cache="$cache_dir" \
  npm_config_ignore_scripts="$ignore_scripts" \
  "$npm_node_bin" "$npm_cli_bin" "${install_args[@]}"
  rm -rf "$cache_dir" || true
}

skip_runtimes_raw="${CTX_BUNDLE_SKIP_RUNTIMES:-}"
skip_images_raw="${CTX_BUNDLE_SKIP_IMAGES:-}"
only_providers_raw="${CTX_BUNDLE_ONLY_PROVIDERS:-}"
only_providers_raw="${only_providers_raw// /}"
skip_providers_raw="${CTX_BUNDLE_SKIP_PROVIDERS:-}"
skip_providers_raw="${skip_providers_raw// /}"

provider_selected_for_bundle() {
  local provider_id="$1"
  if [[ -n "$only_providers_raw" ]]; then
    if [[ ",$only_providers_raw," != *",$provider_id,"* ]]; then
      return 1
    fi
  fi
  if [[ -n "$skip_providers_raw" ]]; then
    if [[ ",$skip_providers_raw," == *",$provider_id,"* ]]; then
      return 1
    fi
  fi
  return 0
}

runtime_need_node="1"
runtime_need_python="1"
if [[ "${CTX_BUNDLE_DEPENDENCY_AWARE_RUNTIMES:-1}" == "1" ]]; then
  read -r runtime_need_node runtime_need_python < <(
    run_python - "$MATRIX_JSON" "$only_providers_raw" "$skip_providers_raw" <<'PY'
import json
import sys

matrix_path = sys.argv[1]
only_raw = sys.argv[2]
skip_raw = sys.argv[3]

only = {v for v in only_raw.split(",") if v}
skip = {v for v in skip_raw.split(",") if v}

def include(provider_id: str) -> bool:
    if only and provider_id not in only:
        return False
    if provider_id in skip:
        return False
    return True

needs_node = False
needs_python = False

with open(matrix_path, "r", encoding="utf-8") as fh:
    data = json.load(fh)

for provider in data.get("providers", []):
    provider_id = provider.get("id", "")
    if not provider_id or not include(provider_id):
        continue
    managed = provider.get("managed_install") or {}
    kind = managed.get("kind")
    if kind == "npm":
        needs_node = True
    elif kind == "python":
        needs_python = True

print("1" if needs_node else "0", "1" if needs_python else "0")
PY
  )
fi

if ! is_falsy "$LOCAL_ADAPTER_MODE"; then
  if provider_selected_for_bundle "amp" \
    || provider_selected_for_bundle "pi" \
    || provider_selected_for_bundle "goose" \
    || provider_selected_for_bundle "openhands"; then
    runtime_need_node="1"
  fi
fi
if provider_selected_for_bundle "claude-crp" || provider_selected_for_bundle "claude-cli"; then
  runtime_need_node="1"
fi
if provider_selected_for_bundle "cline"; then
  runtime_need_node="1"
fi

if provider_selected_for_bundle "claude-crp"; then
  if [[ ! -d "$CLAUDE_CRP_WORKSPACE" ]]; then
    log "error: claude-crp local-only bundling requires workspace at $CLAUDE_CRP_WORKSPACE"
    exit 5
  fi
  if [[ ! -f "$CLAUDE_CRP_WORKSPACE/dist/runtime.js" ]]; then
    if [[ ! -x "$ROOT/scripts/build_claude_crp.sh" ]]; then
      log "error: missing claude-crp build script at $ROOT/scripts/build_claude_crp.sh"
      exit 5
    fi
    if ! command -v pnpm >/dev/null 2>&1; then
      log "error: bundling claude-crp requires pnpm to build local adapter payload"
      exit 5
    fi
  fi
fi

if provider_selected_for_bundle "claude-crp" || provider_selected_for_bundle "claude-cli"; then
  runtime_need_node="1"
fi

if provider_selected_for_bundle "claude-crp"; then
  if [[ ! -d "$CLAUDE_CRP_WORKSPACE" ]]; then
    log "error: claude-crp local-only bundling requires workspace at $CLAUDE_CRP_WORKSPACE"
    exit 5
  fi
  if [[ ! -f "$CLAUDE_CRP_WORKSPACE/dist/runtime.js" ]]; then
    if [[ ! -x "$ROOT/scripts/build_claude_crp.sh" ]]; then
      log "error: missing claude-crp build script at $ROOT/scripts/build_claude_crp.sh"
      exit 5
    fi
    if ! command -v pnpm >/dev/null 2>&1; then
      log "error: bundling claude-crp requires pnpm to build local adapter payload"
      exit 5
    fi
  fi
fi

if ! is_truthy "$skip_runtimes_raw"; then
  if [[ "$runtime_need_node" == "1" ]]; then
    ensure_node_runtime
  fi
  if [[ "$runtime_need_python" == "1" ]]; then
    ensure_python_runtime
  fi
  ensure_podman_runtime
fi
if ! is_falsy "$INCLUDE_BRIDGE"; then
  require_bridge_binary
fi

if is_truthy "$BUILD_LOCAL_ADAPTERS"; then
  if is_falsy "$LOCAL_ADAPTER_MODE"; then
    log "warn: CTX_BUNDLE_BUILD_LOCAL_ADAPTERS set but CTX_BUNDLE_LOCAL_ADAPTERS=off"
  fi
  build_local_adapters
fi

node_root_rel=""
node_bin_rel=""
npm_cli_rel=""
node_root=""
node_bin=""
npm_cli=""

python_root_rel=""
python_bin_rel=""
python_root=""
python_bin=""

if ! is_truthy "$skip_runtimes_raw"; then
  if [[ "$runtime_need_node" == "1" ]]; then
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
  fi

  if [[ "$runtime_need_python" == "1" ]]; then
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
  fi
fi

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
    # If a provider declares releases, respect them: skip managed installs that
    # are not marked supported for this ctx version (e.g. pending/blocked).
    releases = provider.get("releases", [])
    if releases:
        supported = [r for r in releases if r.get("status") == "supported"]
        supported = [r for r in supported if release_matches_context(r, ctx_version)]
        if not supported:
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
  if [[ -s "$local_providers_src" ]]; then
    local filtered
    filtered="$(mktemp /tmp/ctx-bundle-local-providers-filtered.XXXXXX)"
    awk -F $'\x1f' -v id="$provider_id" '$1 != id' "$local_providers_src" > "$filtered"
    mv "$filtered" "$local_providers_src"
  fi
  local sep=$'\x1f'
  printf '%s%s%s%s%s%s%s%s%s%s%s%s%s%s%s%s%s\n' \
    "$provider_id" "$sep" "$kind" "$sep" "$version" "$sep" \
    "$source_path" "$sep" "" "$sep" "$bin_path" "$sep" "" "$sep" "" "$sep" \
    "$args_json" >> "$local_providers_src"
  local found=0
  local existing
  for existing in "${local_ids[@]-}"; do
    if [[ "$existing" == "$provider_id" ]]; then
      found=1
      break
    fi
  done
  if [[ "$found" == "0" ]]; then
    local_ids+=("$provider_id")
  fi
}

if ! is_falsy "$INCLUDE_BRIDGE"; then
  bridge_src="$(local_bridge_binary_path)"
  add_local_provider "acp-crp-bridge" "local-bin" "local" "$bridge_src" "$(basename "$bridge_src")" "[]"
fi

should_build_codex_crp() {
  if ! provider_selected_for_bundle "codex"; then
    return 1
  fi
  if is_truthy "$CODEX_CRP_BUILD_MODE"; then
    return 0
  fi
  if is_falsy "$CODEX_CRP_BUILD_MODE"; then
    return 1
  fi
  log "error: invalid CTX_BUNDLE_BUILD_CODEX_CRP='${CODEX_CRP_BUILD_MODE}' (expected 0/1)"
  exit 5
}

local_codex_crp_binary_path() {
  if [[ ! -d "$CODEX_CRP_WORKSPACE" ]]; then
    return 1
  fi
  local profile="${CTX_BUNDLE_CODEX_CRP_PROFILE:-release}"
  local target_dir="${CTX_BUNDLE_CODEX_CRP_TARGET_DIR:-$bundle_build_dir/codex-crp/${os}/${arch}}"

	local profile_args=()
	if [[ "$profile" == "release" ]]; then
	  profile_args+=(--release)
	elif [[ "$profile" != "debug" ]]; then
	  log "error: invalid CTX_BUNDLE_CODEX_CRP_PROFILE: $profile (expected debug|release)"
	  exit 5
	fi

	# codex-crp release defaults are intentionally very heavy upstream; use a lighter optimized
	# profile so local bundling is practical in CI/dev and consistent with container builds below.
	local -a cargo_profile_env=()
	if [[ "$profile" == "release" ]]; then
	  cargo_profile_env+=(
	    "CARGO_PROFILE_RELEASE_LTO=false"
	    "CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16"
	    "CARGO_PROFILE_RELEASE_OPT_LEVEL=2"
	  )
	fi

	build_codex_crp_in_container() {
	  if ! ensure_docker_ready_for_builds "0" "codex-crp container build"; then
	    log "error: building codex-crp for ${os}/${arch} requires Docker on PATH and a healthy daemon."
	    log "       Ensure Docker Desktop (or Docker Engine) is running and retry."
	    exit 5
	  fi

	  mkdir -p "$target_dir"

	  local platform="linux/arm64"
	  if [[ "$arch" == "x86_64" ]]; then
	    platform="linux/amd64"
	  fi

	  local image="${CTX_BUNDLE_RUST_IMAGE:-rust:1}"

	  local -a run_args
	  run_args=(run --rm --platform "$platform" -v "$CODEX_CRP_WORKSPACE:/work:rw" -v "$target_dir:/target:rw" -w /work -e CARGO_TARGET_DIR=/target)

	  # Avoid `bash -l` here: login shells can reset PATH and drop Cargo.
	  #
	  # Also: `codex-crp` upstream ships with a very heavy release profile (fat LTO, 1 codegen unit),
	  # which can OOM on typical Docker Desktop configs. Override to a lighter release build: still
	  # optimized, but much less memory hungry.
	  docker "${run_args[@]}" "$image" bash -c "set -euo pipefail; export PATH=\"/usr/local/cargo/bin:\$PATH\"; rustup target add '$rust_target' >/dev/null 2>&1 || true; ${cargo_profile_env[*]} cargo build -p codex-crp --target '$rust_target' ${profile_args[*]}"
	}

	if [[ "$os" == "linux" && "$host_os" != "linux" ]]; then
	  build_codex_crp_in_container
	else
	  require_cmd cargo
	  (
	    cd "$CODEX_CRP_WORKSPACE"
	    env CARGO_TARGET_DIR="$target_dir" "${cargo_profile_env[@]}" cargo build -p codex-crp --target "$rust_target" "${profile_args[@]}"
	  )
	fi

	local bin="$target_dir/$rust_target/$profile/codex-crp$BIN_EXT"
	if [[ ! -f "$bin" ]]; then
	  log "error: codex-crp binary not found at $bin"
	  exit 5
	fi
	printf '%s' "$bin"
}

should_bundle_local_claude_crp() {
  if ! provider_selected_for_bundle "claude-crp"; then
    return 1
  fi
  if [[ ! -d "$CLAUDE_CRP_WORKSPACE" ]]; then
    log "error: claude-crp local-only bundling requires workspace at $CLAUDE_CRP_WORKSPACE"
    exit 5
  fi
  return 0
}

ensure_local_claude_crp_dist() {
  local dist_entry="$CLAUDE_CRP_WORKSPACE/dist/runtime.js"
  if [[ -f "$dist_entry" ]]; then
    return 0
  fi
  if ! should_bundle_local_claude_crp; then
    return 1
  fi
  if [[ ! -x "$ROOT/scripts/build_claude_crp.sh" ]]; then
    log "error: missing claude-crp build script at $ROOT/scripts/build_claude_crp.sh"
    return 1
  fi
  if ! command -v pnpm >/dev/null 2>&1; then
    log "error: bundling claude-crp requires pnpm to build local adapter payload"
    return 1
  fi
  if ! "$ROOT/scripts/build_claude_crp.sh"; then
    log "error: failed to build local claude-crp bundle payload"
    return 1
  fi
  if [[ ! -f "$dist_entry" ]]; then
    log "error: claude-crp build completed without dist entrypoint at $dist_entry"
    return 1
  fi
  return 0
}

if should_build_codex_crp; then
  codex_crp_version="$(get_matrix_version "codex")"
  if [[ -z "$codex_crp_version" ]]; then
    codex_crp_version="local"
  fi
  codex_crp_bin="$(local_codex_crp_binary_path || true)"
  if [[ -n "$codex_crp_bin" && -f "$codex_crp_bin" ]]; then
    add_local_provider "codex" "local-bin" "$codex_crp_version" "$codex_crp_bin" "codex-crp$BIN_EXT" "[]"
  else
    log "error: codex-crp build requested but source not available at $CODEX_CRP_WORKSPACE"
    exit 5
  fi
fi

if should_bundle_local_claude_crp; then
  claude_crp_version="$(get_matrix_version "claude-crp")"
  if [[ -z "$claude_crp_version" ]]; then
    claude_crp_version="$(package_json_version "$CLAUDE_CRP_WORKSPACE/package.json")"
  fi
  if [[ -z "$claude_crp_version" ]]; then
    claude_crp_version="local"
  fi

  if ! ensure_local_claude_crp_dist; then
    log "error: failed to prepare local claude-crp bundle payload at $CLAUDE_CRP_WORKSPACE/dist/runtime.js"
    exit 5
  fi
  if [[ ! -f "$CLAUDE_CRP_WORKSPACE/bin/claude-crp" ]]; then
    log "error: local claude-crp entrypoint missing at $CLAUDE_CRP_WORKSPACE/bin/claude-crp"
    exit 5
  fi
  add_local_provider "claude-crp" "local-node" "$claude_crp_version" "$CLAUDE_CRP_WORKSPACE" "bin/claude-crp" "[]"
fi

if ! is_falsy "$LOCAL_ADAPTER_MODE"; then
  local_adapter_required=0
  if is_truthy "$LOCAL_ADAPTER_MODE"; then
    local_adapter_required=1
  fi

  adapter_version_override="${CTX_BUNDLE_ADAPTER_VERSION:-}"
  for id in amp pi goose openhands droid; do
    if ! provider_selected_for_bundle "$id"; then
      continue
    fi
    version="$adapter_version_override"
    if [[ -z "$version" ]]; then
      version="$(get_matrix_version "$id")"
    fi
    if [[ -z "$version" ]]; then
      version="local"
    fi

    if [[ "$id" == "amp" || "$id" == "pi" || "$id" == "goose" || "$id" == "openhands" ]]; then
      src="$(local_adapter_node_entrypoint "$id")"
      if [[ ! -f "$src" ]]; then
        dir="$(local_adapter_dir "$id")"
        if [[ -n "$dir" && -d "$LOCAL_ADAPTERS_DIR/$dir" ]]; then
          if command -v pnpm >/dev/null 2>&1; then
            (cd "$LOCAL_ADAPTERS_DIR/$dir" && pnpm install --ignore-scripts)
            (cd "$LOCAL_ADAPTERS_DIR/$dir" && pnpm run build)
          else
            require_cmd npm
            (cd "$LOCAL_ADAPTERS_DIR/$dir" && npm install --ignore-scripts)
            (cd "$LOCAL_ADAPTERS_DIR/$dir" && npm run build)
          fi
          src="$(local_adapter_node_entrypoint "$id")"
        fi
      fi
      if [[ -f "$src" ]]; then
        adapter_root="$(local_adapter_root_path "$id" || true)"
        if [[ -n "$adapter_root" && "$src" == "$adapter_root/"* ]]; then
          entrypoint_rel="${src#"$adapter_root/"}"
          add_local_provider "$id" "local-node" "$version" "$adapter_root" "$entrypoint_rel" "[]"
        else
          add_local_provider "$id" "local-node" "$version" "$src" "$(basename "$src")" "[]"
        fi
      elif [[ "$local_adapter_required" == "1" ]]; then
        log "error: missing local-node adapter entrypoint for $id at $src"
        exit 5
      fi
      continue
    fi

    src="$(local_adapter_binary_path "$id" || true)"
    if [[ ! -f "$src" ]]; then
      dir="$(local_adapter_dir "$id")"
      if [[ -n "$dir" && -d "$LOCAL_ADAPTERS_DIR/$dir" ]]; then
        require_cmd cargo
        (cd "$LOCAL_ADAPTERS_DIR/$dir" && cargo build --release --target "$rust_target")
        src="$(local_adapter_binary_path "$id" || true)"
      fi
    fi
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

while IFS=$'\x1f' read -r provider_id kind version url archive bin_path package entrypoint args_json; do
  if [[ -z "$provider_id" || -z "$kind" ]]; then
    continue
  fi
  include_provider=0
  if provider_selected_for_bundle "$provider_id"; then
    include_provider=1
  elif [[ "$provider_id" == "claude-cli" ]] && provider_selected_for_bundle "claude-crp"; then
    include_provider=1
  fi
  if [[ "$include_provider" != "1" ]]; then
    continue
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
      rm -f "$dest"
      cp "$url" "$dest"
      if [[ "$os" != "windows" ]]; then
        chmod +x "$dest" || true
      fi
      maybe_adhoc_codesign_macos_binary "$dest"
      echo "$version" > "$version_marker"
      command_path="$dest"
      ;;
    local-node)
      if is_truthy "$skip_runtimes_raw"; then
        log "error: cannot bundle local-node provider $provider_id when CTX_BUNDLE_SKIP_RUNTIMES=1"
        exit 5
      fi
      if [[ -z "$url" ]]; then
        log "error: missing local adapter entrypoint for $provider_id"
        exit 5
      fi
      if [[ -z "$bin_path" ]]; then
        bin_path="$(basename "$url")"
      fi

      if [[ -d "$url" ]]; then
        src_entrypoint="$url/$bin_path"
        if [[ ! -f "$src_entrypoint" ]]; then
          log "error: local-node entrypoint missing for $provider_id: $src_entrypoint"
          exit 5
        fi
        copy_local_node_adapter_payload "$url" "$provider_root"
        if [[ -f "$provider_root/package.json" ]]; then
          npm_install_bundle "$provider_root" "" "project"
        fi
        prune_provider_node_payload "$provider_id" "$provider_root"
      else
        mkdir -p "$provider_root"
        dest="$provider_root/$bin_path"
        mkdir -p "$(dirname "$dest")"
        rm -f "$dest"
        cp "$url" "$dest"
      fi

      dest="$provider_root/$bin_path"
      if [[ ! -f "$dest" ]]; then
        log "error: bundled local-node entrypoint missing for $provider_id: $dest"
        exit 5
      fi
      if [[ -z "$node_bin" || ! -f "$node_bin" ]]; then
        log "error: local-node provider $provider_id requires bundled node runtime"
        exit 5
      fi
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
          dmg)
            if [[ "$os" != "macos" ]]; then
              log "error: archive type 'dmg' for $provider_id requires macOS host tooling"
              exit 5
            fi
            require_cmd hdiutil
            dmg_mount_dir="$(mktemp -d "/tmp/ctx-dmg-${provider_id}.XXXXXX")"
            if ! hdiutil attach -nobrowse -readonly -mountpoint "$dmg_mount_dir" "$tmp_file" >/dev/null; then
              rm -rf "$dmg_mount_dir" "$tmp_file"
              log "error: failed to mount dmg for $provider_id"
              exit 5
            fi
            if ! copy_dmg_payload_without_external_symlinks "$dmg_mount_dir" "$provider_root"; then
              hdiutil detach "$dmg_mount_dir" -force >/dev/null 2>&1 || true
              rm -rf "$dmg_mount_dir" "$tmp_file"
              log "error: failed to copy dmg payload for $provider_id"
              exit 5
            fi
            hdiutil detach "$dmg_mount_dir" -force >/dev/null 2>&1 || true
            rm -rf "$dmg_mount_dir" "$tmp_file"
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
      if [[ "$provider_id" == "cline" ]]; then
        write_cline_bundle_stubs "$provider_root"
        patch_cline_standalone_runtime "$provider_root"
        if [[ -z "$node_bin" || ! -f "$node_bin" ]]; then
          log "error: cline provider requires bundled node runtime"
          exit 5
        fi
        entrypoint_path="$provider_root/dist-standalone/cline-acp.js"
        if [[ ! -f "$entrypoint_path" ]]; then
          log "error: cline standalone entrypoint missing: $entrypoint_path"
          exit 5
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
      fi
      maybe_adhoc_codesign_macos_binary "$command_path"
      ;;
    npm)
      if is_truthy "$skip_runtimes_raw"; then
        log "error: cannot bundle npm provider $provider_id when CTX_BUNDLE_SKIP_RUNTIMES=1"
        exit 5
      fi
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
      prune_provider_node_payload "$provider_id" "$provider_root"
      if [[ -z "$node_bin" || ! -f "$node_bin" ]]; then
        log "error: npm provider $provider_id requires bundled node runtime"
        exit 5
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
      if is_truthy "$skip_runtimes_raw"; then
        log "error: cannot bundle python provider $provider_id when CTX_BUNDLE_SKIP_RUNTIMES=1"
        exit 5
      fi
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
        if [[ -z "$python_bin" || ! -f "$python_bin" ]]; then
          log "error: python provider $provider_id requires bundled python runtime"
          exit 5
        fi
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
images_out="$(mktemp /tmp/ctx-bundle-images-out.XXXXXX)"

if ! is_truthy "$skip_runtimes_raw"; then
  if [[ "$runtime_need_node" == "1" ]]; then
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
  fi

  if [[ "$runtime_need_python" == "1" ]]; then
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
  fi

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
fi

bundle_harness_image() {
  local image_ref="$1"
  local tar_rel="$2"
  local tar_path="$bundle_dir/$tar_rel"
  local platform="${3:-}"

  mkdir -p "$(dirname "$tar_path")"

  # Do not silently produce a mislabeled tar when cross-bundling (e.g. CTX_BUNDLE_ARCH=x86_64 on arm64).
  if [[ -z "$platform" ]]; then
    log "error: internal: missing platform for harness image bundle"
    exit 2
  fi

  if ! ensure_docker_ready_for_builds "1" "harness image bundling"; then
    exit 2
  fi

  # Build the requested Linux arch and write a docker-archive compatible tar.
  docker buildx build \
    --progress plain \
    --platform "$platform" \
    -t "$image_ref" \
    -f "$ROOT/containers/ctx-harness/Dockerfile" \
    --output "type=docker,dest=$tar_path" \
    "$ROOT"
}

bundle_harness_manifest_entry() {
  local image_ref="$1"
  local image_arch="$2"
  local tar_rel="$3"
  local sha="$4"
  HARNESS_IMAGE_REF_ENV="$image_ref" \
  HARNESS_IMAGE_VERSION_ENV="$image_ref" \
  HARNESS_IMAGE_OS_ENV="linux" \
  HARNESS_IMAGE_ARCH_ENV="$image_arch" \
  HARNESS_IMAGE_SHA_ENV="$sha" \
  HARNESS_IMAGE_TAR_REL_ENV="$tar_rel" \
  run_python - <<'PY' >> "$images_out"
import json
import os

entry = {
    "id": "ctx-harness",
    "version": os.environ["HARNESS_IMAGE_VERSION_ENV"],
    "os": os.environ["HARNESS_IMAGE_OS_ENV"],
    "arch": os.environ["HARNESS_IMAGE_ARCH_ENV"],
    "sha256": os.environ["HARNESS_IMAGE_SHA_ENV"],
    "tar": os.environ["HARNESS_IMAGE_TAR_REL_ENV"],
    "image": os.environ["HARNESS_IMAGE_REF_ENV"],
}
print(json.dumps(entry, separators=(",", ":")))
PY
}

bundle_harness_mode="${CTX_BUNDLE_HARNESS_IMAGE:-}"
bundle_harness_mode="${bundle_harness_mode// /}"
if is_truthy "$skip_images_raw"; then
  bundle_harness_mode=""
fi
if is_truthy "${bundle_harness_mode:-}"; then
  HARNESS_IMAGE_REF="$(read_const DEFAULT_CONTAINER_IMAGE "$ROOT/core/crates/ctx-http/src/harness_runtime.rs")"
  harness_tar_rel="images/ctx-harness-linux-${arch}.tar"
  harness_platform="linux/amd64"
  if [[ "$arch" == "aarch64" ]]; then
    harness_platform="linux/arm64"
  fi
  bundle_harness_image "$HARNESS_IMAGE_REF" "$harness_tar_rel" "$harness_platform"
  harness_sha="$(sha256_file "$bundle_dir/$harness_tar_rel")"
  bundle_harness_manifest_entry "$HARNESS_IMAGE_REF" "$arch" "$harness_tar_rel" "$harness_sha"
elif [[ "$bundle_harness_mode" == "both" || "$bundle_harness_mode" == "all" ]]; then
  HARNESS_IMAGE_REF="$(read_const DEFAULT_CONTAINER_IMAGE "$ROOT/core/crates/ctx-http/src/harness_runtime.rs")"

  tar_x86="images/ctx-harness-linux-x86_64.tar"
  tar_arm="images/ctx-harness-linux-aarch64.tar"

  bundle_harness_image "$HARNESS_IMAGE_REF" "$tar_x86" "linux/amd64"
  sha_x86="$(sha256_file "$bundle_dir/$tar_x86")"
  bundle_harness_manifest_entry "$HARNESS_IMAGE_REF" "x86_64" "$tar_x86" "$sha_x86"

  bundle_harness_image "$HARNESS_IMAGE_REF" "$tar_arm" "linux/arm64"
  sha_arm="$(sha256_file "$bundle_dir/$tar_arm")"
  bundle_harness_manifest_entry "$HARNESS_IMAGE_REF" "aarch64" "$tar_arm" "$sha_arm"
fi

manifest_path="$bundle_dir/manifest.json"
GENERATED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
PROVIDERS_OUT="$providers_out" \
RUNTIMES_OUT="$runtimes_out" \
IMAGES_OUT="$images_out" \
GENERATED_AT_ENV="$GENERATED_AT" \
MANIFEST_PATH="$manifest_path" \
BUNDLE_APPEND_ENV="${CTX_BUNDLE_APPEND:-}" \
run_python - <<'PY'
import json
import os
from pathlib import Path

providers_path = Path(os.environ["PROVIDERS_OUT"])
runtimes_path = Path(os.environ["RUNTIMES_OUT"])
images_path = Path(os.environ["IMAGES_OUT"])
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

images = []
if images_path.exists():
    for line in images_path.read_text().splitlines():
        if line.strip():
            images.append(json.loads(line))

manifest = {
    "version": 1,
    "generated_at": os.environ["GENERATED_AT_ENV"],
    "providers": providers,
    "runtimes": runtimes,
    "images": images,
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
        images_by_key = {}
        for entry in existing.get("images", []) or []:
            key = (entry.get("id"), entry.get("os"), entry.get("arch"))
            images_by_key[key] = entry
        for entry in images:
            key = (entry.get("id"), entry.get("os"), entry.get("arch"))
            images_by_key[key] = entry
        manifest["images"] = sorted(
            images_by_key.values(),
            key=lambda e: (e.get("id", ""), e.get("os", ""), e.get("arch", "")),
        )

manifest_path.write_text(json.dumps(manifest, indent=2))
PY

rm -f "$providers_src" "$providers_out" "$runtimes_out" "$local_providers_src" || true

echo "$bundle_dir"
