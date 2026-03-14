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

build_local_adapters() {
  if [[ ! -d "$LOCAL_ADAPTERS_DIR" ]]; then
    log "error: local adapters dir missing: $LOCAL_ADAPTERS_DIR"
    exit 4
  fi

  local node_adapter_id
  for node_adapter_id in amp pi; do
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
  tmp="$(mktemp -p "$dest_dir" "node-${node_folder}.XXXXXX")"
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

python_runtime_root_rel() {
  local python_version="${1:-$PYTHON_VERSION}"
  local python_build_tag="${2:-$PYTHON_BUILD_TAG}"
  printf '%s' "runtimes/python/${os}/${arch}/cpython-${python_version}+${python_build_tag}-${python_target}"
}

python_runtime_bin_rel() {
  local py_root_rel="$1"
  if [[ "$os" == "windows" ]]; then
    printf '%s' "python.exe"
  else
    if [[ -f "$bundle_dir/$py_root_rel/bin/python3" ]]; then
      printf '%s' "bin/python3"
    else
      printf '%s' "bin/python"
    fi
  fi
}

ensure_python_runtime_versioned() {
  local python_version="$1"
  local python_build_tag="$2"
  local py_root_rel
  py_root_rel="$(python_runtime_root_rel "$python_version" "$python_build_tag")"
  local py_root="$bundle_dir/$py_root_rel"
  local py_bin_rel
  py_bin_rel="$(python_runtime_bin_rel "$py_root_rel")"
  local py_bin="$py_root/$py_bin_rel"

  if [[ -f "$py_bin" ]]; then
    return
  fi

  local dest_dir
  dest_dir="$(dirname "$py_root")"
  mkdir -p "$dest_dir"
  local py_folder
  py_folder="$(basename "$py_root")"
  local asset="cpython-${python_version}+${python_build_tag}-${python_target}-install_only.tar.gz"
  local url="https://github.com/indygreg/python-build-standalone/releases/download/${python_build_tag}/${asset}"
  local tmp
  tmp="$(mktemp -p "$dest_dir" "python-${py_folder}.XXXXXX")"
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

  py_bin_rel="$(python_runtime_bin_rel "$py_root_rel")"
  py_bin="$py_root/$py_bin_rel"
  if [[ ! -f "$py_bin" ]]; then
    log "error: python runtime incomplete after extract"
    exit 4
  fi
}

ensure_python_runtime() {
  ensure_python_runtime_versioned "$PYTHON_VERSION" "$PYTHON_BUILD_TAG"
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
  local archive_type=""
  if [[ -n "$PODMAN_ARCHIVE_PATH" ]]; then
    archive_path="$PODMAN_ARCHIVE_PATH"
    case "$archive_path" in
      *.zip) archive_type="zip" ;;
      *.tar) archive_type="tar" ;;
      *.tgz|*.tar.gz) archive_type="tar.gz" ;;
      *)
        log "error: unsupported podman archive type: $archive_path"
        exit 4
        ;;
    esac
  elif [[ -n "$PODMAN_ARCHIVE_URL" ]]; then
    local dest_dir
    dest_dir="$(dirname "$podman_root_abs")"
    mkdir -p "$dest_dir"
    archive_type="tar.gz"
    if [[ "$PODMAN_ARCHIVE_URL" == *.zip ]]; then
      archive_type="zip"
    elif [[ "$PODMAN_ARCHIVE_URL" == *.tar ]]; then
      archive_type="tar"
    elif [[ "$PODMAN_ARCHIVE_URL" == *.tgz ]]; then
      archive_type="tgz"
    fi
    archive_path="$(mktemp -p "$dest_dir" "podman-${PODMAN_VERSION}.XXXXXX")"
    fetch_file "$PODMAN_ARCHIVE_URL" "$archive_path"
  else
    log "error: PODMAN_ARCHIVE_URL or PODMAN_ARCHIVE_PATH is required when CTX_BUNDLE_PODMAN=1"
    exit 3
  fi
  verify_sha256_if_expected "$archive_path" "$PODMAN_ARCHIVE_SHA256" "podman archive"

  local extract_dir
  extract_dir="$(mktemp -d "$(dirname "$podman_root_abs")/podman-${PODMAN_VERSION}.extract.XXXXXX")"
  case "$archive_type" in
    zip)
      require_cmd unzip
      unzip -q "$archive_path" -d "$extract_dir"
      ;;
    tar)
      require_cmd tar
      tar -xf "$archive_path" -C "$extract_dir"
      ;;
    tgz|tar.gz)
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

# Provider-specific packaging is isolated from the shared bootstrap/runtime setup above.
# shellcheck source=scripts/lib/bundled_harnesses_providers.sh
source "$ROOT/scripts/lib/bundled_harnesses_providers.sh"

# Harness image bundling and final manifest emission are release-owned concerns.
# shellcheck source=scripts/lib/bundled_harnesses_images.sh
source "$ROOT/scripts/lib/bundled_harnesses_images.sh"
