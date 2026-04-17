#!/usr/bin/env bash
set -euo pipefail

print_selected_packages=0
case "${1:-}" in
  -h|--help)
  cat <<'EOF'
Usage: scripts/install_desktop_deps_linux_ubuntu.sh

Installs system packages required to build the Tauri v2 desktop app on Ubuntu/Debian.

Options:
  --print-selected-packages  Resolve the package set and print it without running apt-get.

This script intentionally does NOT install:
  - Rust toolchain (rustup/cargo)
  - Node.js / pnpm

EOF
  exit 0
  ;;
  --print-selected-packages)
    print_selected_packages=1
    ;;
  "")
    ;;
  *)
    echo "error: unknown argument: ${1}" >&2
    exit 2
    ;;
esac

if [[ "$print_selected_packages" != "1" ]] && ! command -v apt-get >/dev/null 2>&1; then
  echo "error: apt-get not found; this script targets Ubuntu/Debian." >&2
  exit 1
fi

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
if [[ "$print_selected_packages" != "1" ]]; then
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
  echo "error: could not find libwebkit2gtk-4.1-dev (required by current Tauri Linux stack)." >&2
  exit 3
}

libsoup_pkg="$(choose_first_available_pkg libsoup-3.0-dev)" || {
  echo "error: could not find libsoup-3.0-dev (required by current Tauri Linux stack)." >&2
  exit 3
}

webkit_driver_pkg="$(choose_first_available_pkg webkit2gtk-driver)" || {
  echo "error: could not find webkit2gtk-driver (required to provide WebKitWebDriver for tauri-driver Linux automation)." >&2
  exit 3
}

appindicator_pkg="$(choose_first_available_pkg libayatana-appindicator3-dev libappindicator3-dev)" || {
  echo "error: could not find an appindicator dev package (tried libayatana-appindicator3-dev, libappindicator3-dev)." >&2
  exit 3
}

fuse_runtime_pkg="$(choose_first_available_pkg libfuse2t64 libfuse2)" || {
  echo "error: could not find an AppImage FUSE runtime package (tried libfuse2t64, libfuse2)." >&2
  exit 3
}

selected_packages=(
  "${packages[@]}"
  "$webkit_pkg"
  "$webkit_driver_pkg"
  "$libsoup_pkg"
  "$appindicator_pkg"
  "$fuse_runtime_pkg"
)

if [[ "$print_selected_packages" == "1" ]]; then
  printf '%s\n' "${selected_packages[@]}"
  exit 0
fi

run_apt_get_with_lock_retry install -y --no-install-recommends \
  "${selected_packages[@]}"

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
    if docker buildx version 2>/dev/null | grep -Fq "${DOCKER_BUILDX_VERSION#v}"; then
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

install_docker_buildx_plugin

echo
echo "${BOLD}Sanity check (pkg-config)${RESET}"
for pc in glib-2.0 gtk+-3.0 libsoup-3.0 javascriptcoregtk-4.1 webkit2gtk-4.1 libcap; do
  if pkg-config --exists "${pc}"; then
    echo "- ${pc}: OK"
  else
    echo "- ${pc}: MISSING (check packages and pkg-config search path)" >&2
    exit 2
  fi
done

if command -v xdg-mime >/dev/null 2>&1; then
  echo "- xdg-mime: OK ($(command -v xdg-mime))"
else
  echo "- xdg-mime: MISSING (install xdg-utils)" >&2
  exit 2
fi

for cmd in xauth xvfb-run; do
  if command -v "$cmd" >/dev/null 2>&1; then
    echo "- $cmd: OK ($(command -v "$cmd"))"
  else
    echo "- $cmd: MISSING (install xauth/xvfb for Linux desktop automation)" >&2
    exit 2
  fi
done

for cmd in desktop-file-validate mksquashfs zsyncmake patchelf appstreamcli gtk-update-icon-cache; do
  if command -v "$cmd" >/dev/null 2>&1; then
    echo "- $cmd: OK ($(command -v "$cmd"))"
  else
    echo "- $cmd: MISSING (install corresponding AppImage packaging deps)" >&2
    exit 2
  fi
done

if docker buildx version >/dev/null 2>&1; then
  echo "- docker buildx: OK"
else
  echo "- docker buildx: MISSING (install pinned Docker buildx CLI plugin)" >&2
  exit 2
fi

if command -v WebKitWebDriver >/dev/null 2>&1; then
  echo "- WebKitWebDriver: OK ($(command -v WebKitWebDriver))"
else
  echo "- WebKitWebDriver: MISSING (install webkit2gtk-driver)" >&2
  exit 2
fi

if command -v ldconfig >/dev/null 2>&1 && ldconfig -p | grep -q 'libfuse\.so\.2'; then
  echo "- libfuse.so.2: OK"
else
  echo "- libfuse.so.2: MISSING (install ${fuse_runtime_pkg})" >&2
  exit 2
fi

echo
echo "${BOLD}Next steps${RESET}"
cat <<'EOF'
- Ensure prerequisites: Rust toolchain + Node.js + pnpm
- Install JS deps:    pnpm -C core install
- Prep web + daemons: pnpm -C core desktop:prep
- Run desktop dev:    pnpm -C core/apps/desktop dev
EOF
