#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage:
  scripts/release_runtime_install_smoke.sh \
    --daemon-bin <path-to-ctx-daemon-binary> \
    --app <path-to-ctx.app> \
    [--provider <provider-id>] \
    [--target <install-target>] \
    [--bind <host:port>]

Notes:
  - Starts daemon with a fresh data dir and staged bundle resources.
  - Verifies runtime install endpoint can start an install and accept cancel.
  - Fails hard on invalid/unsupported/matrix-mismatch target errors.
USAGE
}

daemon_bin=""
app_path=""
provider_id="codex"
install_target="host"
bind_addr=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --daemon-bin)
      daemon_bin="${2:-}"
      shift 2
      ;;
    --app)
      app_path="${2:-}"
      shift 2
      ;;
    --provider)
      provider_id="${2:-}"
      shift 2
      ;;
    --target)
      install_target="${2:-}"
      shift 2
      ;;
    --bind)
      bind_addr="${2:-}"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "error: unknown argument: $1" >&2
      usage
      exit 1
      ;;
  esac
done

if [[ -z "$daemon_bin" || -z "$app_path" ]]; then
  echo "error: --daemon-bin and --app are required" >&2
  usage
  exit 1
fi

if [[ -z "$bind_addr" ]]; then
  bind_addr="127.0.0.1:$((43000 + RANDOM % 1000))"
fi

if [[ ! -x "$daemon_bin" ]]; then
  echo "error: daemon binary not executable: $daemon_bin" >&2
  exit 1
fi

bundle_dir="$app_path/Contents/Resources/bundles"
if [[ ! -d "$bundle_dir" ]]; then
  echo "error: missing bundles dir in app: $bundle_dir" >&2
  exit 1
fi

if ! command -v jq >/dev/null 2>&1; then
  echo "error: jq is required" >&2
  exit 1
fi
if ! command -v curl >/dev/null 2>&1; then
  echo "error: curl is required" >&2
  exit 1
fi

tmp_root="$(mktemp -d "${TMPDIR:-/tmp}/ctx-runtime-install-smoke.XXXXXX")"
data_dir="$tmp_root/data"
log_path="$tmp_root/daemon.log"
mkdir -p "$data_dir"

daemon_pid=""
cleanup() {
  if [[ -n "$daemon_pid" ]] && kill -0 "$daemon_pid" >/dev/null 2>&1; then
    kill "$daemon_pid" >/dev/null 2>&1 || true
    wait "$daemon_pid" >/dev/null 2>&1 || true
  fi
  rm -rf "$tmp_root" >/dev/null 2>&1 || true
}
trap cleanup EXIT

echo "smoke: starting daemon on $bind_addr"
CTX_BUNDLE_DIR="$bundle_dir" \
CTX_DEV_MODE=1 \
"$daemon_bin" serve --bind "$bind_addr" --data-dir "$data_dir" >"$log_path" 2>&1 &
daemon_pid="$!"

health_url="http://$bind_addr/health"
for _ in {1..80}; do
  if curl -fsS "$health_url" >/dev/null 2>&1; then
    break
  fi
  sleep 0.25
done
if ! curl -fsS "$health_url" >/dev/null 2>&1; then
  echo "error: daemon did not become healthy; logs:" >&2
  cat "$log_path" >&2 || true
  exit 1
fi

providers_url="http://$bind_addr/api/providers?target=$install_target"
providers_json="$(curl -fsS "$providers_url")"

if ! jq -e --arg id "$provider_id" '.[] | select(.provider_id == $id)' <<<"$providers_json" >/dev/null; then
  provider_id="$(jq -r '.[] | select(.details.install_supported == "true") | .provider_id' <<<"$providers_json" | head -n 1)"
fi
if [[ -z "$provider_id" ]]; then
  echo "error: no install-supported provider found for target=$install_target" >&2
  exit 1
fi

supported="$(jq -r --arg id "$provider_id" '.[] | select(.provider_id == $id) | .details.install_supported // "false"' <<<"$providers_json")"
if [[ "$supported" != "true" ]]; then
  echo "error: provider '$provider_id' is not install-supported for target=$install_target" >&2
  exit 1
fi

start_url="http://$bind_addr/api/providers/$provider_id/install?target=$install_target"
start_json="$(curl -fsS -X POST "$start_url")"
install_id="$(jq -r '.install_id // empty' <<<"$start_json")"
if [[ -z "$install_id" || "$install_id" == "null" ]]; then
  echo "error: failed to start provider install for $provider_id: $start_json" >&2
  exit 1
fi

cancel_url="http://$bind_addr/api/providers/install/$install_id/cancel"
curl -fsS -X POST "$cancel_url" >/dev/null

info_url="http://$bind_addr/api/providers/install/$install_id"
state=""
error_code=""
for _ in {1..30}; do
  info_json="$(curl -fsS "$info_url")"
  state="$(jq -r '.state // ""' <<<"$info_json")"
  error_code="$(jq -r '.error_code // ""' <<<"$info_json")"
  if [[ "$state" != "running" ]]; then
    break
  fi
  sleep 0.3
done

if [[ "$state" == "failed" && ( "$error_code" == "invalid_target" || "$error_code" == "unsupported_target" || "$error_code" == "matrix_mismatch" ) ]]; then
  echo "error: runtime install smoke failed with actionable regression code '$error_code' (provider=$provider_id target=$install_target)" >&2
  echo "$info_json" >&2
  exit 1
fi

if [[ "$state" != "cancelled" && "$state" != "succeeded" && "$state" != "failed" ]]; then
  echo "error: unexpected install state after cancel smoke: '$state'" >&2
  echo "$info_json" >&2
  exit 1
fi

echo "ok: runtime install smoke passed (provider=$provider_id target=$install_target state=$state error_code=${error_code:-none})"
