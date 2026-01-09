#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

: "${MAC_HOST:?MAC_HOST must be set to the remote Mac host}"
MAC_USER="${MAC_USER:-admin}"
MAC_ROOT="${MAC_ROOT:-/Users/${MAC_USER}/ctx-sync}"
SSH_OPTS="${SSH_OPTS:--o StrictHostKeyChecking=accept-new}"

AUTO_INSTALL="${AUTO_INSTALL:-1}"
AUTO_ACCEPT_OPEN="${AUTO_ACCEPT_OPEN:-1}"
BUILD_WAIT_SECONDS="${BUILD_WAIT_SECONDS:-180}"
APP_LAUNCH_WAIT_SECONDS="${APP_LAUNCH_WAIT_SECONDS:-12}"
SIMULATOR_NAME="${SIMULATOR_NAME:-}"
METRO_PORT="${METRO_PORT:-8081}"

REMOTE="${MAC_USER}@${MAC_HOST}"
REMOTE_CORE="$MAC_ROOT/core"
REMOTE_LOG_DIR="$MAC_ROOT/logs"
REMOTE_SCREENSHOT_DIR="$MAC_ROOT/screenshots"

APPSCRIPT_LOCAL="$SCRIPT_DIR/ios_sim_tap_open.applescript"
APPSCRIPT_REMOTE="$MAC_ROOT/ios_sim_tap_open.applescript"
SIM_TAP_APP_REMOTE="$MAC_ROOT/SimTap.app"

timestamp="$(date +%Y%m%d-%H%M%S)"
REMOTE_BUILD_LOG="$REMOTE_LOG_DIR/ios-build-$timestamp.log"
REMOTE_METRO_LOG="$REMOTE_LOG_DIR/metro-$timestamp.log"
REMOTE_SCREENSHOT="$REMOTE_SCREENSHOT_DIR/ios-sim-$timestamp.png"
LOCAL_SCREENSHOT="${LOCAL_SCREENSHOT:-/tmp/ios-sim-$timestamp.png}"

run_remote() {
  ssh $SSH_OPTS "$REMOTE" "$@"
}

require_remote_tool() {
  local tool="$1"
  if run_remote "command -v \"$tool\" >/dev/null 2>&1"; then
    return 0
  fi
  if [[ "$AUTO_INSTALL" != "1" ]]; then
    echo "Missing $tool on $MAC_HOST. Install it or rerun with AUTO_INSTALL=1."
    exit 1
  fi
  case "$tool" in
    node)
      run_remote "brew install node"
      ;;
    pnpm)
      run_remote "npm install -g pnpm@9.15.1"
      ;;
    pod)
      run_remote "brew install cocoapods"
      ;;
    *)
      echo "Auto-install not configured for $tool."
      exit 1
      ;;
  esac
}

start_metro() {
  if run_remote "command -v lsof >/dev/null 2>&1"; then
    if run_remote "lsof -i tcp:$METRO_PORT -sTCP:LISTEN >/dev/null 2>&1"; then
      return 0
    fi
  else
    if run_remote "pgrep -f 'expo.*start' >/dev/null 2>&1"; then
      return 0
    fi
  fi

  run_remote "cd \"$REMOTE_CORE\" && EXPO_NO_INTERACTIVE=1 nohup pnpm -C apps/mobile start -- --dev-client --host lan --port $METRO_PORT > \"$REMOTE_METRO_LOG\" 2>&1 & echo \\$! > \"$MAC_ROOT/metro.pid\""
}

"$SCRIPT_DIR/mac_sync.sh"

require_remote_tool node
require_remote_tool pnpm
require_remote_tool pod

run_remote "mkdir -p \"$REMOTE_LOG_DIR\" \"$REMOTE_SCREENSHOT_DIR\""

needs_install=0
if ! run_remote "test -d \"$REMOTE_CORE/node_modules/.pnpm\""; then
  needs_install=1
fi

remote_lock_hash="$(run_remote "shasum -a 256 \"$REMOTE_CORE/pnpm-lock.yaml\" | awk '{print \$1}'")"
remote_stamp_hash="$(run_remote "cat \"$MAC_ROOT/.pnpm-lock.sha\" 2>/dev/null || true")"
if [[ "$needs_install" -eq 1 || "$remote_lock_hash" != "$remote_stamp_hash" ]]; then
  run_remote "cd \"$REMOTE_CORE\" && pnpm install --filter mobile... --frozen-lockfile"
  run_remote "printf '%s' \"$remote_lock_hash\" > \"$MAC_ROOT/.pnpm-lock.sha\""
fi

start_metro

if [[ -n "$SIMULATOR_NAME" ]]; then
  SIMULATOR_UDID="$(run_remote "xcrun simctl list devices available | grep -m1 -F \"$SIMULATOR_NAME\" | sed -E 's/.*\(([0-9A-F-]+)\).*/\\1/'")"
else
  sim_line="$(run_remote "xcrun simctl list devices available | grep -m1 -E 'iPhone'")"
  SIMULATOR_NAME="$(echo "$sim_line" | sed -E 's/^ *([^()]+).*/\1/' | xargs)"
  SIMULATOR_UDID="$(echo "$sim_line" | sed -E 's/.*\(([0-9A-F-]+)\).*/\1/')"
fi

if [[ -z "${SIMULATOR_UDID:-}" ]]; then
  echo "No available iPhone simulator found on $MAC_HOST."
  exit 1
fi

run_remote "xcrun simctl boot \"$SIMULATOR_UDID\" >/dev/null 2>&1 || true"
run_remote "open -a Simulator >/dev/null 2>&1 || true"

run_remote "cd \"$REMOTE_CORE\" && EXPO_NO_INTERACTIVE=1 nohup pnpm mobile:ios -- --device \"$SIMULATOR_NAME\" --no-bundler > \"$REMOTE_BUILD_LOG\" 2>&1 &"

sleep "$BUILD_WAIT_SECONDS"

remote_ip="$(run_remote "ipconfig getifaddr en0 2>/dev/null || ipconfig getifaddr en1 2>/dev/null || ifconfig | awk '/inet / && \$2 != \"127.0.0.1\" {print \$2; exit}'")"
if [[ -n "$remote_ip" ]]; then
  dev_url="com.anonymous.mobile://expo-development-client/?url=http%3A%2F%2F${remote_ip}%3A${METRO_PORT}"
  run_remote "xcrun simctl openurl \"$SIMULATOR_UDID\" \"$dev_url\""

  if [[ "$AUTO_ACCEPT_OPEN" == "1" ]]; then
    scp -q "$APPSCRIPT_LOCAL" "$REMOTE:$APPSCRIPT_REMOTE"
    run_remote "if ! command -v osacompile >/dev/null 2>&1; then echo 'osacompile not found'; exit 1; fi"
    run_remote "if [[ ! -d \"$SIM_TAP_APP_REMOTE\" ]]; then osacompile -o \"$SIM_TAP_APP_REMOTE\" \"$APPSCRIPT_REMOTE\"; fi"
    run_remote "open -a \"$SIM_TAP_APP_REMOTE\"" || echo "Auto-accept failed; enable Accessibility for the SimTap app on the mini."
  fi
fi

sleep "$APP_LAUNCH_WAIT_SECONDS"

run_remote "xcrun simctl io \"$SIMULATOR_UDID\" screenshot \"$REMOTE_SCREENSHOT\""
scp -q "$REMOTE:$REMOTE_SCREENSHOT" "$LOCAL_SCREENSHOT"

echo "Screenshot saved to $LOCAL_SCREENSHOT"
echo "Remote build log: $REMOTE_BUILD_LOG"
echo "Remote metro log: $REMOTE_METRO_LOG"
