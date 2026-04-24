#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'USAGE'
Usage:
  scripts/release_runtime_install_smoke.sh \
    --daemon-bin <path-to-ctx-daemon-binary> \
    [--app <path-to-desktop-app-root>] \
    [--bundle-dir <path-to-bundles-dir>] \
    [--provider <provider-id>] \
    [--all-providers] \
    [--complete] \
    [--target <install-target>] \
    [--bind <host:port>] \
    [--timeout-seconds <seconds>]

Notes:
  - Starts daemon with a fresh data dir and staged bundle resources.
  - `--app` supports macOS `.app` roots and extracted Linux AppImage roots.
  - Verifies runtime install endpoint can start an install and accept cancel by default.
  - With --complete, waits for the selected provider install to complete successfully.
  - With --all-providers, waits for every install-supported provider to complete successfully.
  - Fails hard on invalid/unsupported/matrix-mismatch target errors.
USAGE
}

daemon_bin=""
app_path=""
bundle_dir=""
provider_id="codex-crp"
install_target="host"
bind_addr=""
all_providers="0"
complete_install="0"
timeout_seconds="600"

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
    --bundle-dir)
      bundle_dir="${2:-}"
      shift 2
      ;;
    --provider)
      provider_id="${2:-}"
      shift 2
      ;;
    --all-providers)
      all_providers="1"
      complete_install="1"
      shift
      ;;
    --complete)
      complete_install="1"
      shift
      ;;
    --target)
      install_target="${2:-}"
      shift 2
      ;;
    --bind)
      bind_addr="${2:-}"
      shift 2
      ;;
    --timeout-seconds)
      timeout_seconds="${2:-}"
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

if [[ -z "$daemon_bin" ]]; then
  echo "error: --daemon-bin is required" >&2
  usage
  exit 1
fi

if [[ -z "$app_path" && -z "$bundle_dir" ]]; then
  echo "error: either --app or --bundle-dir is required" >&2
  usage
  exit 1
fi

if ! [[ "$timeout_seconds" =~ ^[0-9]+$ ]] || [[ "$timeout_seconds" -le 0 ]]; then
  echo "error: --timeout-seconds must be a positive integer" >&2
  exit 1
fi

if [[ -z "$bind_addr" ]]; then
  bind_addr="127.0.0.1:$((43000 + RANDOM % 1000))"
fi

if [[ ! -x "$daemon_bin" ]]; then
  echo "error: daemon binary not executable: $daemon_bin" >&2
  exit 1
fi

if [[ -z "$bundle_dir" ]]; then
  if [[ -d "$app_path/Contents/Resources/bundles" ]]; then
    bundle_dir="$app_path/Contents/Resources/bundles"
  elif [[ -d "$app_path/usr/lib/ctx/bundles" ]]; then
    bundle_dir="$app_path/usr/lib/ctx/bundles"
  elif [[ -d "$app_path/bundles" ]]; then
    bundle_dir="$app_path/bundles"
  else
    echo "error: could not resolve bundles dir from app root: $app_path" >&2
    exit 1
  fi
fi

if [[ ! -d "$bundle_dir" ]]; then
  echo "error: missing bundles dir: $bundle_dir" >&2
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

health_url="http://$bind_addr/api/health"
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

auth_path="$data_dir/daemon_auth.json"
if [[ ! -f "$auth_path" ]]; then
  echo "error: missing daemon auth file: $auth_path" >&2
  cat "$log_path" >&2 || true
  exit 1
fi
auth_token="$(jq -r '.token // empty' "$auth_path")"
if [[ -z "$auth_token" ]]; then
  echo "error: daemon auth token missing in $auth_path" >&2
  cat "$log_path" >&2 || true
  exit 1
fi
auth_header=( -H "Authorization: Bearer $auth_token" )

providers_url="http://$bind_addr/api/providers?target=$install_target"
providers_json="$(curl -fsS "${auth_header[@]}" "$providers_url")"

provider_ids=()
if [[ "$all_providers" == "1" ]]; then
  while IFS= read -r id; do
    [[ -z "$id" ]] && continue
    provider_ids+=("$id")
  done < <(jq -r '.[] | select(.details.install_supported == "true") | .provider_id' <<<"$providers_json" | sort -u)
else
  if ! jq -e --arg id "$provider_id" '.[] | select(.provider_id == $id)' <<<"$providers_json" >/dev/null; then
    provider_id="$(jq -r '.[] | select(.details.install_supported == "true") | .provider_id' <<<"$providers_json" | head -n 1)"
  fi
  [[ -n "$provider_id" ]] && provider_ids+=("$provider_id")
fi

if [[ "${#provider_ids[@]}" -eq 0 ]]; then
  echo "error: no install-supported provider found for target=$install_target" >&2
  exit 1
fi

for current_provider_id in "${provider_ids[@]}"; do
  supported="$(jq -r --arg id "$current_provider_id" '.[] | select(.provider_id == $id) | .details.install_supported // "false"' <<<"$providers_json")"
  if [[ "$supported" != "true" ]]; then
    echo "error: provider '$current_provider_id' is not install-supported for target=$install_target" >&2
    exit 1
  fi

  start_url="http://$bind_addr/api/providers/$current_provider_id/install?target=$install_target"
  start_json="$(curl -fsS "${auth_header[@]}" -X POST "$start_url")"
  install_id="$(jq -r '.install_id // empty' <<<"$start_json")"
  if [[ -z "$install_id" || "$install_id" == "null" ]]; then
    echo "error: failed to start provider install for $current_provider_id: $start_json" >&2
    exit 1
  fi

  info_url="http://$bind_addr/api/providers/install/$install_id"
  state=""
  error_code=""
  info_json="{}"

  if [[ "$complete_install" == "1" ]]; then
    deadline=$((SECONDS + timeout_seconds))
    while true; do
      info_json="$(curl -fsS "${auth_header[@]}" "$info_url")"
      state="$(jq -r '.state // ""' <<<"$info_json")"
      error_code="$(jq -r '.error_code // ""' <<<"$info_json")"
      if [[ "$state" == "succeeded" || "$state" == "failed" || "$state" == "cancelled" ]]; then
        break
      fi
      if (( SECONDS >= deadline )); then
        echo "error: provider install timed out after ${timeout_seconds}s (provider=$current_provider_id target=$install_target state=${state:-unknown})" >&2
        echo "$info_json" >&2
        exit 1
      fi
      sleep 1
    done

    if [[ "$state" != "succeeded" ]]; then
      echo "error: provider install did not succeed (provider=$current_provider_id target=$install_target state=$state error_code=${error_code:-none})" >&2
      echo "$info_json" >&2
      exit 1
    fi
  else
    cancel_url="http://$bind_addr/api/providers/install/$install_id/cancel"
    curl -fsS "${auth_header[@]}" -X POST "$cancel_url" >/dev/null

    for _ in {1..30}; do
      info_json="$(curl -fsS "${auth_header[@]}" "$info_url")"
      state="$(jq -r '.state // ""' <<<"$info_json")"
      error_code="$(jq -r '.error_code // ""' <<<"$info_json")"
      if [[ "$state" != "running" ]]; then
        break
      fi
      sleep 0.3
    done

    if [[ "$state" == "failed" && ( "$error_code" == "invalid_target" || "$error_code" == "unsupported_target" || "$error_code" == "matrix_mismatch" ) ]]; then
      echo "error: runtime install smoke failed with actionable regression code '$error_code' (provider=$current_provider_id target=$install_target)" >&2
      echo "$info_json" >&2
      exit 1
    fi

    if [[ "$state" != "cancelled" && "$state" != "succeeded" && "$state" != "failed" ]]; then
      echo "error: unexpected install state after cancel smoke: '$state'" >&2
      echo "$info_json" >&2
      exit 1
    fi
  fi

  echo "ok: runtime install smoke passed (provider=$current_provider_id target=$install_target state=$state error_code=${error_code:-none})"
done
