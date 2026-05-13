#!/usr/bin/env bash

updater_local_smoke_fail() {
  echo "error: $*" >&2
  return 1
}

normalize_local_smoke_app_permissions() {
  local app_path="${1:?local smoke app path is required}"

  if [[ -f "$app_path" && ! -x "$app_path" ]]; then
    chmod +x "$app_path" || true
  fi

  case "$app_path" in
    *.AppDir/AppRun | */squashfs-root/AppRun)
      local appdir_path="${app_path%/AppRun}"
      local wrapped_path="${app_path}.wrapped"
      if [[ -f "$wrapped_path" && ! -x "$wrapped_path" ]]; then
        chmod +x "$wrapped_path" || true
      fi
      if [[ -d "$appdir_path/usr/bin" ]]; then
        while IFS= read -r bundled_bin; do
          [[ -n "$bundled_bin" ]] || continue
          if [[ ! -x "$bundled_bin" ]]; then
            chmod +x "$bundled_bin" || true
          fi
        done < <(find "$appdir_path/usr/bin" -maxdepth 1 -type f 2>/dev/null | LC_ALL=C sort)
      fi
      while IFS= read -r bundled_helper; do
        [[ -n "$bundled_helper" ]] || continue
        if [[ ! -x "$bundled_helper" ]]; then
          chmod +x "$bundled_helper" || true
        fi
      done < <(
        find "$appdir_path" -type f \( -name 'WebKit*Process' -o -name 'chrome-sandbox' \) 2>/dev/null | LC_ALL=C sort
      )
      ;;
  esac
}

validate_linux_local_smoke_app_path() {
  local app_path="${1:?linux app path is required}"

  if [[ ! -f "$app_path" ]]; then
    updater_local_smoke_fail "linux local smoke path must be a file: $app_path"
    return 1
  fi

  if [[ "$app_path" == *.AppImage ]]; then
    updater_local_smoke_fail \
      "linux local smoke must launch the AppDir AppRun launcher, not the outer AppImage wrapper: $app_path"
    return 1
  fi

  case "$app_path" in
    *.AppDir/AppRun | */squashfs-root/AppRun)
      return 0
      ;;
    *.AppDir/usr/bin/* | */squashfs-root/usr/bin/*)
      updater_local_smoke_fail \
        "linux local smoke must launch the AppDir/AppImage AppRun launcher, not an inner bundled binary: $app_path"
      return 1
      ;;
  esac

  updater_local_smoke_fail \
    "linux local smoke requires a bundled AppDir/AppImage AppRun launcher (*.AppDir/AppRun or */squashfs-root/AppRun): $app_path"
  return 1
}

resolve_linux_local_smoke_app_path() {
  local root="${1:?repo root is required}"
  local bundle_dir="${root}/core/apps/desktop/src-tauri/target/release/bundle/appimage"
  local -a matches=()
  local match

  if [[ ! -d "$bundle_dir" ]]; then
    updater_local_smoke_fail \
      "linux local smoke expected an AppImage bundle directory at $bundle_dir"
    return 1
  fi

  while IFS= read -r match; do
    [[ -n "$match" ]] || continue
    matches+=("$match")
  done < <(
    find "$bundle_dir" -maxdepth 3 -type f -path '*.AppDir/AppRun' 2>/dev/null | LC_ALL=C sort
  )

  case "${#matches[@]}" in
    1)
      validate_linux_local_smoke_app_path "${matches[0]}" || return 1
      printf '%s\n' "${matches[0]}"
      return 0
      ;;
    0)
      updater_local_smoke_fail \
        "linux local smoke expected exactly one bundled AppDir AppRun launcher under $bundle_dir (*.AppDir/AppRun)"
      return 1
      ;;
  esac

  updater_local_smoke_fail \
    "linux local smoke found multiple bundled AppDir AppRun launchers under $bundle_dir"
  printf '%s\n' "${matches[@]}" >&2
  return 1
}

resolve_local_smoke_app_path() {
  local root="${1:?repo root is required}"
  local os_name="${2:-$(uname -s)}"
  local override="${CTX_DESKTOP_APP_PATH:-}"

  if [[ -n "$override" ]]; then
    if [[ ! -e "$override" ]]; then
      updater_local_smoke_fail "CTX_DESKTOP_APP_PATH override does not exist: $override"
      return 1
    fi

    if [[ "$os_name" == "Linux" ]]; then
      validate_linux_local_smoke_app_path "$override" || return 1
    fi

    printf '%s\n' "$override"
    return 0
  fi

  if [[ "$os_name" == "Darwin" ]]; then
    local release_app_dir="${root}/core/apps/desktop/src-tauri/target/release/bundle/macos"
    if [[ -d "$release_app_dir" ]]; then
      local app
      app="$(find "$release_app_dir" -maxdepth 1 -type d -name '*.app' | head -n1 || true)"
      if [[ -n "$app" && -d "$app" ]]; then
        printf '%s\n' "$app"
        return 0
      fi
    fi
    return 1
  fi

  if [[ "$os_name" == "Linux" ]]; then
    resolve_linux_local_smoke_app_path "$root"
    return $?
  fi

  local generic="${root}/core/apps/desktop/src-tauri/target/release/ctx"
  if [[ -e "$generic" ]]; then
    printf '%s\n' "$generic"
    return 0
  fi
  return 1
}
