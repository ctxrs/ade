export function renderUninstallScript() {
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

need_cmd uname
need_cmd rm
need_cmd id

assume_yes="\${CTX_UNINSTALL_YES:-0}"
while [ "$#" -gt 0 ]; do
  case "$1" in
    --yes|-y)
      assume_yes=1
      ;;
    *)
      fail "unknown argument: $1"
      ;;
  esac
  shift
done

os="$(uname -s)"
uid="$(id -u)"
home_dir="$HOME"
data_dir="\${CTX_DATA_DIR:-$home_dir/.ctx}"

mac_system_app="\${CTX_UNINSTALL_MAC_SYSTEM_APP:-/Applications/ctx.app}"
mac_user_app="\${CTX_UNINSTALL_MAC_USER_APP:-$home_dir/Applications/ctx.app}"
mac_avf_private_dir="\${CTX_UNINSTALL_AVF_PRIVATE_DIR:-$home_dir/.ctx-avf-host-private}"
mac_avf_socket_dir="\${CTX_UNINSTALL_AVF_SOCKET_DIR:-/tmp/ctxavf-uid-$uid}"
mac_app_support_dir="\${CTX_UNINSTALL_MAC_APP_SUPPORT_DIR:-$home_dir/Library/Application Support/rs.ctx.desktop}"
mac_preferences_file="\${CTX_UNINSTALL_MAC_PREFERENCES_FILE:-$home_dir/Library/Preferences/rs.ctx.desktop.plist}"
mac_webkit_dir="\${CTX_UNINSTALL_MAC_WEBKIT_DIR:-$home_dir/Library/WebKit/rs.ctx.desktop}"

linux_install_dir="\${CTX_INSTALL_DIR:-$home_dir/.local/share/ctx}"
linux_bin_dir="\${CTX_BIN_DIR:-$home_dir/.local/bin}"
linux_launcher_path="\${CTX_UNINSTALL_LINUX_LAUNCHER_PATH:-$linux_bin_dir/ctx-desktop}"
linux_desktop_entry_path="\${CTX_UNINSTALL_LINUX_DESKTOP_ENTRY_PATH:-\${XDG_DATA_HOME:-$home_dir/.local/share}/applications/ctx.desktop}"
linux_debian_package_name="\${CTX_UNINSTALL_DEBIAN_PACKAGE_NAME:-ctx}"

remove_path() {
  target="$1"
  if [ -L "$target" ] || [ -e "$target" ]; then
    rm -rf "$target"
    log "Removed $target"
  fi
}

remove_path_as_root() {
  target="$1"
  if [ -L "$target" ] || [ -e "$target" ]; then
    run_as_root rm -rf "$target"
    log "Removed $target"
  fi
}

linux_os_release_path() {
  if [ -n "\${CTX_UNINSTALL_OS_RELEASE_PATH:-}" ]; then
    printf '%s\\n' "$CTX_UNINSTALL_OS_RELEASE_PATH"
    return 0
  fi
  printf '/etc/os-release\\n'
}

linux_os_release_field() {
  field_name="$1"
  release_path="$(linux_os_release_path)"
  if [ ! -r "$release_path" ]; then
    printf '\\n'
    return 0
  fi
  awk -F= -v target="$field_name" '
    $1 == target {
      value = $2
      gsub(/^"/, "", value)
      gsub(/"$/, "", value)
      print tolower(value)
      exit
    }
  ' "$release_path"
}

linux_is_debian_like() {
  os_id="$(linux_os_release_field ID)"
  os_like="$(linux_os_release_field ID_LIKE)"
  case " $os_id $os_like " in
    *" debian "*|*" ubuntu "*) return 0 ;;
    *) return 1 ;;
  esac
}

run_as_root() {
  if [ "$(id -u)" = "0" ]; then
    "$@"
    return 0
  fi
  need_cmd sudo
  sudo "$@"
}

debian_package_installed() {
  package_name="$1"
  if command -v dpkg-query >/dev/null 2>&1; then
    status="$(dpkg-query -W -f='\${Status}' "$package_name" 2>/dev/null || true)"
    [ "$status" = "install ok installed" ]
    return
  fi
  if command -v dpkg >/dev/null 2>&1; then
    dpkg -s "$package_name" >/dev/null 2>&1
    return
  fi
  return 1
}

print_targets() {
  case "$os" in
    Darwin)
      printf '%s\\n' \
        "$mac_system_app" \
        "$mac_user_app" \
        "$data_dir" \
        "$mac_avf_private_dir" \
        "$mac_avf_socket_dir" \
        "$mac_app_support_dir" \
        "$mac_preferences_file" \
        "$mac_webkit_dir"
      ;;
    Linux)
      printf '%s\\n' \
        "$data_dir" \
        "$linux_install_dir" \
        "$linux_launcher_path" \
        "$linux_desktop_entry_path"
      if linux_is_debian_like; then
        printf 'Debian package: %s (if installed)\\n' "$linux_debian_package_name"
      fi
      ;;
  esac
}

confirm_uninstall() {
  if [ "$assume_yes" = "1" ]; then
    return 0
  fi
  if ! exec 3<>/dev/tty 2>/dev/null; then
    fail "interactive confirmation requires /dev/tty; rerun with --yes for noninteractive use"
  fi
  {
    printf 'ctx uninstall will remove:\\n'
    print_targets
    printf '\\nProceed? [y/N] '
  } >&3
  IFS= read -r answer <&3 || fail "unable to read confirmation from /dev/tty"
  exec 3>&-
  case "$answer" in
    y|Y|yes|YES) ;;
    *)
      log "Cancelled."
      exit 1
      ;;
  esac
}

uninstall_macos() {
  if command -v osascript >/dev/null 2>&1; then
    osascript -e 'quit app "ctx"' >/dev/null 2>&1 || true
  fi
  remove_path_as_root "$mac_system_app"
  remove_path "$mac_user_app"
  remove_path "$data_dir"
  remove_path "$mac_avf_private_dir"
  remove_path "$mac_avf_socket_dir"
  remove_path "$mac_app_support_dir"
  remove_path "$mac_preferences_file"
  remove_path "$mac_webkit_dir"
}

uninstall_linux() {
  if linux_is_debian_like && command -v apt-get >/dev/null 2>&1; then
    if debian_package_installed "$linux_debian_package_name"; then
      log "Removing Debian package $linux_debian_package_name"
      run_as_root apt-get purge -y "$linux_debian_package_name"
    fi
  fi
  remove_path "$data_dir"
  remove_path "$linux_install_dir"
  remove_path "$linux_launcher_path"
  remove_path "$linux_desktop_entry_path"
}

confirm_uninstall

case "$os" in
  Darwin)
    uninstall_macos
    ;;
  Linux)
    uninstall_linux
    ;;
  MINGW*|MSYS*|CYGWIN*)
    fail "windows uninstall script is not available yet"
    ;;
  *)
    fail "unsupported operating system: $os"
    ;;
esac

log "ctx uninstall complete."
`;
}
