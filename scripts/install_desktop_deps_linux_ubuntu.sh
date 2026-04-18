#!/usr/bin/env bash
set -euo pipefail

print_selected_packages=0
check_only=0
json_output=0
while [[ "$#" -gt 0 ]]; do
  case "${1:-}" in
  -h|--help)
    cat <<'EOF'
Usage: scripts/install_desktop_deps_linux_ubuntu.sh

Installs system packages required to build the Tauri v2 desktop app on Ubuntu/Debian.

Options:
  --print-selected-packages  Resolve the package set and print it without running apt-get.
  --check                    Probe the current runner and fail if requirements are missing.
  --json                     Emit machine-readable JSON (supported with --check or --print-selected-packages).

This script intentionally does NOT install:
  - Rust toolchain (rustup/cargo)
  - Node.js / pnpm

EOF
    exit 0
    ;;
    --check)
      check_only=1
      ;;
    --json)
      json_output=1
      ;;
    --print-selected-packages)
      print_selected_packages=1
      ;;
    *)
      echo "error: unknown argument: ${1}" >&2
      exit 2
      ;;
  esac
  shift
done

if [[ "$print_selected_packages" == "1" && "$check_only" == "1" ]]; then
  echo "error: --print-selected-packages and --check are mutually exclusive" >&2
  exit 2
fi

if [[ "$json_output" == "1" && "$print_selected_packages" != "1" && "$check_only" != "1" ]]; then
  echo "error: --json requires --check or --print-selected-packages" >&2
  exit 2
fi

if [[ "$print_selected_packages" != "1" && "$check_only" != "1" ]] && ! command -v apt-get >/dev/null 2>&1; then
  echo "error: apt-get not found; this script targets Ubuntu/Debian." >&2
  exit 1
fi

MISSING_REQUIREMENTS=()

record_missing_requirement() {
  local requirement_id="$1"
  local message="$2"
  if [[ "$check_only" == "1" ]]; then
    MISSING_REQUIREMENTS+=("${requirement_id}|${message}")
    return 0
  fi
  echo "$message" >&2
  exit 2
}

apt_package_available() {
  local pkg="$1"
  local available="${CTX_TEST_APT_CACHE_AVAILABLE_PACKAGES:-}"
  if [[ -n "$available" ]]; then
    local candidate
    for candidate in $available; do
      if [[ "$candidate" == "$pkg" ]]; then
        return 0
      fi
    done
    return 1
  fi
  apt-cache show "$pkg" >/dev/null 2>&1
}

configure_apt_network() {
  if [[ "${CTX_BUILDKITE_APT_FORCE_IPV4:-0}" != "1" ]]; then
    return 0
  fi
  local apt_conf_dir="${CTX_TEST_APT_CONF_DIR:-/etc/apt/apt.conf.d}"
  "${SUDO[@]}" mkdir -p "$apt_conf_dir"
  "${SUDO[@]}" tee "${apt_conf_dir}/99ctx-force-ipv4" >/dev/null <<'EOF'
Acquire::ForceIPv4 "true";
EOF
}

apt_log_indicates_lock_contention() {
  local log_path="$1"
  grep -Eiq \
    'Could not get lock|Unable to acquire the dpkg frontend lock|Unable to lock directory /var/lib/apt/lists/|Waiting for cache lock' \
    "$log_path"
}

run_apt_get_with_lock_retry() {
  local attempt=1
  local log_path=""

  while true; do
    log_path="$(mktemp "${TMPDIR:-/tmp}/ctx-apt-get.XXXXXX")"
    if "${SUDO[@]}" env DEBIAN_FRONTEND=noninteractive apt-get "$@" >"$log_path" 2>&1; then
      cat "$log_path"
      rm -f "$log_path"
      return 0
    fi

    if apt_log_indicates_lock_contention "$log_path" && (( attempt < APT_LOCK_RETRY_ATTEMPTS )); then
      cat "$log_path" >&2
      echo "warn: apt/dpkg lock is busy (attempt ${attempt}/${APT_LOCK_RETRY_ATTEMPTS}); retrying in ${APT_LOCK_RETRY_SLEEP_SECONDS}s..." >&2
      rm -f "$log_path"
      sleep "$APT_LOCK_RETRY_SLEEP_SECONDS"
      attempt=$((attempt + 1))
      continue
    fi

    cat "$log_path" >&2
    rm -f "$log_path"
    return 1
  done
}

choose_first_available_pkg() {
  local pkg
  for pkg in "$@"; do
    if apt_package_available "$pkg"; then
      printf '%s' "$pkg"
      return 0
    fi
  done
  return 1
}

if [[ -t 1 ]]; then
  BOLD=$'\033[1m'
  RESET=$'\033[0m'
else
  BOLD=""
  RESET=""
fi

readonly DOCKER_BUILDX_VERSION="${DOCKER_BUILDX_VERSION:-v0.30.1}"
readonly APT_LOCK_RETRY_ATTEMPTS="${CTX_APT_LOCK_RETRY_ATTEMPTS:-30}"
readonly APT_LOCK_RETRY_SLEEP_SECONDS="${CTX_APT_LOCK_RETRY_SLEEP_SECONDS:-5}"

packages=(
  build-essential
  binutils
  pkg-config
  curl
  sshpass
  file
  xdg-utils
  xauth
  xvfb
  desktop-file-utils
  squashfs-tools
  zsync
  patchelf
  appstream
  libgtk-3-bin
  libglib2.0-dev
  libgtk-3-dev
  librsvg2-dev
  libssl-dev
  libcap-dev
  liblzma-dev
  musl
  libc++1
  tk
  unzip
)

SUDO=()
if [[ "$print_selected_packages" != "1" && "$check_only" != "1" ]]; then
  if [[ "$(id -u)" -ne 0 ]]; then
    if ! command -v sudo >/dev/null 2>&1; then
      echo "error: sudo not found and not running as root." >&2
      exit 1
    fi

    if [[ -t 0 ]]; then
      SUDO=("sudo")
    else
      if sudo -n true >/dev/null 2>&1; then
        SUDO=("sudo" "-n")
      else
        cat >&2 <<'EOF'
error: this shell is non-interactive and sudo requires a password.

Run this script from an interactive terminal (so sudo can prompt), or run it as root.
EOF
        exit 1
      fi
    fi
  fi

  echo "${BOLD}Installing desktop (Tauri v2) Linux build dependencies (Ubuntu/Debian)${RESET}"

  configure_apt_network
  run_apt_get_with_lock_retry update
fi

webkit_pkg="$(choose_first_available_pkg libwebkit2gtk-4.1-dev)" || {
  webkit_pkg="libwebkit2gtk-4.1-dev"
  record_missing_requirement "apt-package:${webkit_pkg}" "error: could not find libwebkit2gtk-4.1-dev (required by current Tauri Linux stack)."
}

libsoup_pkg="$(choose_first_available_pkg libsoup-3.0-dev)" || {
  libsoup_pkg="libsoup-3.0-dev"
  record_missing_requirement "apt-package:${libsoup_pkg}" "error: could not find libsoup-3.0-dev (required by current Tauri Linux stack)."
}

webkit_driver_pkg="$(choose_first_available_pkg webkit2gtk-driver)" || {
  webkit_driver_pkg="webkit2gtk-driver"
  record_missing_requirement "apt-package:${webkit_driver_pkg}" "error: could not find webkit2gtk-driver (required to provide WebKitWebDriver for tauri-driver Linux automation)."
}

appindicator_pkg="$(choose_first_available_pkg libayatana-appindicator3-dev libappindicator3-dev)" || {
  appindicator_pkg="libayatana-appindicator3-dev"
  record_missing_requirement "apt-package:appindicator" "error: could not find an appindicator dev package (tried libayatana-appindicator3-dev, libappindicator3-dev)."
}

fuse_runtime_pkg="$(choose_first_available_pkg libfuse2t64 libfuse2)" || {
  fuse_runtime_pkg="libfuse2"
  record_missing_requirement "apt-package:libfuse2" "error: could not find an AppImage FUSE runtime package (tried libfuse2t64, libfuse2)."
}

selected_packages=(
  "${packages[@]}"
  "$webkit_pkg"
  "$webkit_driver_pkg"
  "$libsoup_pkg"
  "$appindicator_pkg"
  "$fuse_runtime_pkg"
)

emit_json_summary() {
  local mode="$1"
  SELECTED_PACKAGES="$(printf '%s\n' "${selected_packages[@]}")" \
  MISSING_REQUIREMENTS_TEXT="$(printf '%s\n' "${MISSING_REQUIREMENTS[@]}")" \
  SUMMARY_MODE="$mode" \
  python3 <<'PY'
import json
import os

selected_packages = [
    line for line in os.environ.get("SELECTED_PACKAGES", "").splitlines()
    if line.strip()
]
missing_requirements = []
for entry in os.environ.get("MISSING_REQUIREMENTS_TEXT", "").splitlines():
    if not entry.strip():
        continue
    requirement_id, _, message = entry.partition("|")
    missing_requirements.append({
        "id": requirement_id,
        "message": message,
    })
payload = {
    "mode": os.environ.get("SUMMARY_MODE", ""),
    "selected_packages": selected_packages,
    "missing_requirements": missing_requirements,
    "status": "missing" if missing_requirements else "ok",
}
print(json.dumps(payload, indent=2))
PY
}

if [[ "$print_selected_packages" == "1" ]]; then
  if [[ "$json_output" == "1" ]]; then
    emit_json_summary "print-selected-packages"
    exit 0
  fi
  printf '%s\n' "${selected_packages[@]}"
  exit 0
fi

if [[ "$check_only" != "1" ]]; then
  run_apt_get_with_lock_retry install -y --no-install-recommends \
    "${selected_packages[@]}"
fi

install_docker_buildx_plugin() {
  local arch=""
  case "$(uname -m)" in
    x86_64|amd64)
      arch="amd64"
      ;;
    aarch64|arm64)
      arch="arm64"
      ;;
    *)
      echo "error: unsupported docker buildx architecture: $(uname -m)" >&2
      exit 4
      ;;
  esac

  if command -v docker >/dev/null 2>&1; then
    local buildx_version_output=""
    buildx_version_output="$(docker buildx version 2>/dev/null || true)"
    if [[ "$buildx_version_output" == *"${DOCKER_BUILDX_VERSION#v}"* ]]; then
      return 0
    fi
  fi

  local plugin_dir="/usr/local/lib/docker/cli-plugins"
  local plugin_path="${plugin_dir}/docker-buildx"
  local download_url="https://github.com/docker/buildx/releases/download/${DOCKER_BUILDX_VERSION}/buildx-${DOCKER_BUILDX_VERSION}.linux-${arch}"

  "${SUDO[@]}" mkdir -p "$plugin_dir"
  "${SUDO[@]}" curl -fsSL "$download_url" -o "$plugin_path"
  "${SUDO[@]}" chmod 0755 "$plugin_path"
}

if [[ "$check_only" != "1" ]]; then
  install_docker_buildx_plugin
fi

if [[ "$check_only" == "1" && "$json_output" == "1" ]]; then
  exec 3>&1
  exec 1>&2
fi

echo
echo "${BOLD}Sanity check (pkg-config)${RESET}"
for pc in glib-2.0 gtk+-3.0 libsoup-3.0 javascriptcoregtk-4.1 webkit2gtk-4.1 libcap; do
  if pkg-config --exists "${pc}"; then
    echo "- ${pc}: OK"
  else
    record_missing_requirement "pkg-config:${pc}" "- ${pc}: MISSING (check packages and pkg-config search path)"
  fi
done

if command -v xdg-mime >/dev/null 2>&1; then
  echo "- xdg-mime: OK ($(command -v xdg-mime))"
else
  record_missing_requirement "command:xdg-mime" "- xdg-mime: MISSING (install xdg-utils)"
fi

for cmd in xauth xvfb-run; do
  if command -v "$cmd" >/dev/null 2>&1; then
    echo "- $cmd: OK ($(command -v "$cmd"))"
  else
    record_missing_requirement "command:${cmd}" "- $cmd: MISSING (install xauth/xvfb for Linux desktop automation)"
  fi
done

for cmd in desktop-file-validate mksquashfs zsyncmake patchelf appstreamcli gtk-update-icon-cache; do
  if command -v "$cmd" >/dev/null 2>&1; then
    echo "- $cmd: OK ($(command -v "$cmd"))"
  else
    record_missing_requirement "command:${cmd}" "- $cmd: MISSING (install corresponding AppImage packaging deps)"
  fi
done

if docker buildx version >/dev/null 2>&1; then
  echo "- docker buildx: OK"
else
  record_missing_requirement "docker:buildx" "- docker buildx: MISSING (install pinned Docker buildx CLI plugin)"
fi

if command -v WebKitWebDriver >/dev/null 2>&1; then
  echo "- WebKitWebDriver: OK ($(command -v WebKitWebDriver))"
else
  record_missing_requirement "command:WebKitWebDriver" "- WebKitWebDriver: MISSING (install webkit2gtk-driver)"
fi

if command -v ldconfig >/dev/null 2>&1; then
  ldconfig_output="$(ldconfig -p 2>/dev/null || true)"
else
  ldconfig_output=""
fi

if [[ "$ldconfig_output" == *"libfuse.so.2"* ]]; then
  echo "- libfuse.so.2: OK"
else
  record_missing_requirement "library:libfuse.so.2" "- libfuse.so.2: MISSING (install ${fuse_runtime_pkg})"
fi

if [[ "$check_only" == "1" ]]; then
  if [[ "$json_output" == "1" ]]; then
    exec 1>&3
    emit_json_summary "check"
  fi
  if [[ "${#MISSING_REQUIREMENTS[@]}" -gt 0 ]]; then
    exit 1
  fi
  exit 0
fi

if [[ -t 1 && -z "${CI:-}" && -z "${BUILDKITE:-}" ]]; then
  echo
  echo "${BOLD}Next steps${RESET}"
  cat <<'EOF'
- Ensure prerequisites: Rust toolchain + Node.js + pnpm
- Install JS deps:    pnpm -C core install
- Prep web + daemons: pnpm -C core desktop:prep
- Run desktop dev:    pnpm -C core/apps/desktop dev
EOF
fi
