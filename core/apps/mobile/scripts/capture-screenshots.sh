#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
MOBILE_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
ROOT_DIR="$(cd "${MOBILE_DIR}/../.." && pwd)"

EXPO_BIN="${EXPO_BIN:-${MOBILE_DIR}/node_modules/.bin/expo}"
OUT_DIR="${OUT_DIR:-${MOBILE_DIR}/shots/swiftui}"
LOG_DIR="${LOG_DIR:-${OUT_DIR}/logs}"
SIM_DEVICE="${SIM_DEVICE:-iPhone 15 Pro}"
WAIT_SECONDS="${WAIT_SECONDS:-14}"
MANUAL="${MANUAL:-0}"
BUNDLE_ID="${BUNDLE_ID:-rs.ctx.mobile}"
DEV_CLIENT_SCHEME="${DEV_CLIENT_SCHEME:-}"
BUNDLER_URL="${BUNDLER_URL:-http://127.0.0.1:8081}"
SIM_UDID=""
BUNDLER_PORT="$(python3 - <<'PY' "${BUNDLER_URL}"
import sys
import urllib.parse

url = urllib.parse.urlparse(sys.argv[1])
print(url.port or 8081)
PY
)"

if [[ ! -x "${EXPO_BIN}" ]]; then
  echo "expo CLI not found at ${EXPO_BIN}."
  echo "Run 'npm install' in ${MOBILE_DIR} or set EXPO_BIN."
  exit 1
fi

mkdir -p "${OUT_DIR}" "${LOG_DIR}"
mkdir -p "${ROOT_DIR}/node_modules"
mkdir -p "${MOBILE_DIR}/node_modules"

EXPO_PID=""

cleanup() {
  if [[ -n "${EXPO_PID}" ]]; then
    kill "${EXPO_PID}" >/dev/null 2>&1 || true
    wait "${EXPO_PID}" >/dev/null 2>&1 || true
    EXPO_PID=""
  fi
  kill_packager
}

trap cleanup EXIT

kill_packager() {
  if command -v lsof >/dev/null 2>&1; then
    local pids
    pids="$(lsof -ti tcp:"${BUNDLER_PORT}" 2>/dev/null || true)"
    if [[ -n "${pids}" ]]; then
      echo "${pids}" | xargs -n 1 kill -9 >/dev/null 2>&1 || true
    fi
  fi
}

resolve_scheme() {
  if [[ -n "${DEV_CLIENT_SCHEME}" ]]; then
    return
  fi
  DEV_CLIENT_SCHEME="$(python3 - <<'PY' "${MOBILE_DIR}/app.json"
import json
import sys

path = sys.argv[1]
with open(path, "r", encoding="utf-8") as f:
    data = json.load(f)

expo = data.get("expo", {})
scheme = expo.get("scheme")
slug = expo.get("slug")
if scheme:
    print(scheme)
elif slug:
    print(f"exp+{slug}")
else:
    print("exp+app")
PY
)"
}

resolve_udid() {
  SIM_UDID="$(python3 - <<'PY' "${SIM_DEVICE}"
import json
import subprocess
import sys

name = sys.argv[1]
raw = subprocess.check_output(["xcrun", "simctl", "list", "devices", "--json"], text=True)
info = json.loads(raw)
booted = None
for runtime, devices in info.get("devices", {}).items():
    for device in devices:
        if device.get("name") == name and device.get("state") == "Booted":
            booted = device.get("udid")
if booted:
    print(booted)
    sys.exit(0)
for runtime, devices in info.get("devices", {}).items():
    for device in devices:
        if device.get("name") == name:
            print(device.get("udid", ""))
            sys.exit(0)
print("")
PY
)"
  if [[ -z "${SIM_UDID}" ]]; then
    echo "Unable to resolve simulator UDID for ${SIM_DEVICE}."
    exit 1
  fi
}

boot_simulator() {
  xcrun simctl boot "${SIM_DEVICE}" >/dev/null 2>&1 || true
  xcrun simctl bootstatus "${SIM_DEVICE}" -b
  resolve_udid
}

ensure_app_installed() {
  if ! xcrun simctl listapps "${SIM_UDID}" | grep -q "\"${BUNDLE_ID}\""; then
    echo "App ${BUNDLE_ID} not installed on ${SIM_DEVICE}."
    echo "Run: npx expo run:ios --device \"${SIM_DEVICE}\" --no-bundler"
    exit 1
  fi
}

start_expo() {
  local screen="$1"
  local log_path="$2"
  kill_packager
  (
    cd "${MOBILE_DIR}"
    EXPO_PUBLIC_SCREEN="${screen}" EXPO_USE_METRO_WORKSPACE_ROOT=1 "${EXPO_BIN}" start --dev-client --clear >"${log_path}" 2>&1
  ) &
  EXPO_PID="$!"
}

wait_for_bundler() {
  local attempts=40
  local url="${BUNDLER_URL}/status"
  local i=0
  while [[ $i -lt $attempts ]]; do
    if command -v curl >/dev/null 2>&1; then
      if curl -sf "${url}" | grep -q "packager-status:running"; then
        return
      fi
    else
      python3 - <<'PY' "${url}" >/dev/null 2>&1 && exit 0
import sys
import urllib.request

with urllib.request.urlopen(sys.argv[1], timeout=1) as resp:
    data = resp.read().decode("utf-8")
    if "packager-status:running" in data:
        sys.exit(0)
PY
    fi
    sleep 1
    i=$((i + 1))
  done
}

encode_url() {
  python3 - <<'PY' "$1"
import sys
import urllib.parse

print(urllib.parse.quote(sys.argv[1], safe=""))
PY
}

open_dev_client() {
  local bundler_url="$1"
  local encoded
  encoded="$(encode_url "${bundler_url}")"
  resolve_scheme
  xcrun simctl launch "${SIM_UDID}" "${BUNDLE_ID}" >/dev/null 2>&1 || true
  xcrun simctl openurl "${SIM_UDID}" "${DEV_CLIENT_SCHEME}://expo-development-client/?url=${encoded}"
}

capture_screen() {
  local screen="$1"
  local filename="$2"
  local log_path="${LOG_DIR}/expo-${screen}.log"

  cleanup
  start_expo "${screen}" "${log_path}"
  wait_for_bundler
  open_dev_client "${BUNDLER_URL}"

  if [[ "${MANUAL}" == "1" ]]; then
    echo "Open screen '${screen}' in the simulator, then press enter to capture ${filename}."
    read -r
  else
    sleep "${WAIT_SECONDS}"
  fi

  xcrun simctl io "${SIM_UDID}" screenshot "${OUT_DIR}/${filename}"
  cleanup
}

boot_simulator
ensure_app_installed

screens=(
  "connection|connection-view.png"
  "task-list|task-list.png"
  "new-task|new-task-view.png"
  "diff-panel|diff-panel.png"
  "chat|chat-view.png"
  "settings|settings-view.png"
  "mobile-access|mobile-access.png"
  "diagnostics|diagnostics.png"
  "workspace-selector|workspace-selector.png"
)

for entry in "${screens[@]}"; do
  IFS="|" read -r screen filename <<< "${entry}"
  echo "Capturing ${filename} (EXPO_PUBLIC_SCREEN=${screen})"
  capture_screen "${screen}" "${filename}"
done

echo "Screenshots saved to ${OUT_DIR}"
