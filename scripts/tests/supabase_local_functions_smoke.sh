#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

tmp="$(mktemp -d /tmp/ctx-supabase-local-functions-test.XXXXXX)"
bin_dir="$tmp/bin"
volatile_tmp="$tmp/volatile"
mkdir -p "$bin_dir" "$volatile_tmp"

cleanup() {
  if [[ "${TEST_KEEP_TMP:-0}" == "1" ]]; then
    echo "keeping test tmp: $tmp" >&2
    return
  fi
  rm -rf "$tmp"
}
trap cleanup EXIT

cat >"$bin_dir/node" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

script="${2:-}"
case "$script" in
  *api_url*)
    printf 'http://127.0.0.1:54321\n'
    ;;
  *service_role_key*)
    printf 'service-role-from-status\n'
    ;;
  *anon_key*)
    printf 'anon-from-status\n'
    ;;
  *)
    echo "unexpected node invocation: $*" >&2
    exit 1
    ;;
esac
EOF
chmod +x "$bin_dir/node"

cat >"$bin_dir/infisical" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

output_file=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --output-file)
      output_file="$2"
      shift 2
      ;;
    *)
      shift
      ;;
  esac
done

if [[ -z "$output_file" ]]; then
  echo "missing --output-file" >&2
  exit 1
fi

cat >"$output_file" <<'ENV'
INFISICAL_SECRET=from-infisical
ENV
EOF
chmod +x "$bin_dir/infisical"

cat >"$bin_dir/supabase" <<'EOF'
#!/usr/bin/env bash
set -euo pipefail

mode_of() {
  if stat -c '%a' "$1" >/dev/null 2>&1; then
    stat -c '%a' "$1"
    return
  fi
  stat -f '%OLp' "$1"
}

if [[ "${1:-}" == "status" && "${2:-}" == "--output" && "${3:-}" == "json" ]]; then
  printf '{"api_url":"http://127.0.0.1:54321","service_role_key":"service-role-from-status","anon_key":"anon-from-status"}\n'
  exit 0
fi

if [[ "${1:-}" == "functions" && "${2:-}" == "serve" ]]; then
  if [[ "${3:-}" != "--env-file" || -z "${4:-}" || "${5:-}" != "--no-verify-jwt" ]]; then
    echo "unexpected functions serve invocation: $*" >&2
    exit 1
  fi

  env_file="$4"
  printf '%s\n' "$env_file" >"$SUPABASE_ENV_FILE_RECORD"
  printf '%s\n' "$(mode_of "$env_file")" >"$SUPABASE_ENV_MODE_RECORD"
  cp "$env_file" "$SUPABASE_ENV_SNAPSHOT"
  env | grep -E '^SUPABASE_(URL|SERVICE_ROLE_KEY|ANON_KEY)=' | sort >"$SUPABASE_RUNTIME_ENV_RECORD"
  if [[ "${SUPABASE_TEST_BACKGROUND:-0}" == "1" ]]; then
    sleep 0.3
  fi
  exit 0
fi

echo "unexpected supabase invocation: $*" >&2
exit 1
EOF
chmod +x "$bin_dir/supabase"

assert_contains() {
  local needle="$1"
  local haystack="$2"
  if ! grep -Fq "$needle" "$haystack"; then
    echo "error: expected '$needle' in $haystack" >&2
    cat "$haystack" >&2
    exit 1
  fi
}

wait_for_path() {
  local target="$1"
  for _ in $(seq 1 40); do
    if [[ -e "$target" ]]; then
      return 0
    fi
    sleep 0.1
  done

  echo "error: timed out waiting for $target" >&2
  exit 1
}

wait_for_absence() {
  local target="$1"
  for _ in $(seq 1 40); do
    if [[ ! -e "$target" ]]; then
      return 0
    fi
    sleep 0.1
  done

  echo "error: timed out waiting for cleanup of $target" >&2
  exit 1
}

custom_env_file="$tmp/functions.env"
cat >"$custom_env_file" <<'EOF'
LOCAL_ONLY=from-env-file
EOF

foreground_env_path="$tmp/foreground.env.path"
foreground_env_mode="$tmp/foreground.env.mode"
foreground_env_snapshot="$tmp/foreground.env.snapshot"
foreground_runtime_env="$tmp/foreground.runtime.env"

env \
  PATH="$bin_dir:$PATH" \
  CTX_VOLATILE_TMPDIR="$volatile_tmp" \
  SUPABASE_FUNCTIONS_ENV="$custom_env_file" \
  SUPABASE_FUNCTIONS_USE_INFISICAL=1 \
  STRIPE_LISTEN=0 \
  INFISICAL_PROJECT_DIR="$tmp" \
  SUPABASE_ENV_FILE_RECORD="$foreground_env_path" \
  SUPABASE_ENV_MODE_RECORD="$foreground_env_mode" \
  SUPABASE_ENV_SNAPSHOT="$foreground_env_snapshot" \
  SUPABASE_RUNTIME_ENV_RECORD="$foreground_runtime_env" \
  ./scripts/supabase_local_functions.sh >"$tmp/foreground.stdout" 2>"$tmp/foreground.stderr"

foreground_merged_env="$(<"$foreground_env_path")"
if [[ "$foreground_merged_env" == "/tmp/ctx-supabase-functions.env" ]]; then
  echo "error: helper reused the legacy predictable merged env path" >&2
  exit 1
fi

case "$foreground_merged_env" in
  "$volatile_tmp"/ctx-supabase-functions.*) ;;
  *)
    echo "error: merged env path escaped the configured temp root: $foreground_merged_env" >&2
    exit 1
    ;;
esac

if [[ "$(<"$foreground_env_mode")" != "600" ]]; then
  echo "error: merged env file permissions were not 600" >&2
  cat "$foreground_env_mode" >&2
  exit 1
fi

assert_contains 'INFISICAL_SECRET=from-infisical' "$foreground_env_snapshot"
assert_contains 'LOCAL_ONLY=from-env-file' "$foreground_env_snapshot"
assert_contains 'SUPABASE_URL=http://127.0.0.1:54321' "$foreground_env_snapshot"
assert_contains 'SUPABASE_SERVICE_ROLE_KEY=service-role-from-status' "$foreground_env_snapshot"
assert_contains 'SUPABASE_ANON_KEY=anon-from-status' "$foreground_env_snapshot"
assert_contains 'SUPABASE_URL=http://127.0.0.1:54321' "$foreground_runtime_env"
assert_contains 'SUPABASE_SERVICE_ROLE_KEY=service-role-from-status' "$foreground_runtime_env"
assert_contains 'SUPABASE_ANON_KEY=anon-from-status' "$foreground_runtime_env"

if [[ -e "$foreground_merged_env" ]]; then
  echo "error: foreground merged env file was not cleaned up" >&2
  exit 1
fi

background_env_path="$tmp/background.env.path"
background_env_mode="$tmp/background.env.mode"
background_env_snapshot="$tmp/background.env.snapshot"
background_runtime_env="$tmp/background.runtime.env"
background_log="$tmp/background.log"

env \
  PATH="$bin_dir:$PATH" \
  CTX_VOLATILE_TMPDIR="$volatile_tmp" \
  SUPABASE_FUNCTIONS_ENV="$custom_env_file" \
  SUPABASE_FUNCTIONS_USE_INFISICAL=1 \
  STRIPE_LISTEN=0 \
  INFISICAL_PROJECT_DIR="$tmp" \
  SUPABASE_TEST_BACKGROUND=1 \
  SUPABASE_FUNCTIONS_LOG="$background_log" \
  SUPABASE_ENV_FILE_RECORD="$background_env_path" \
  SUPABASE_ENV_MODE_RECORD="$background_env_mode" \
  SUPABASE_ENV_SNAPSHOT="$background_env_snapshot" \
  SUPABASE_RUNTIME_ENV_RECORD="$background_runtime_env" \
  ./scripts/supabase_local_functions.sh --background >"$tmp/background.stdout" 2>"$tmp/background.stderr"

wait_for_path "$background_env_path"
background_merged_env="$(<"$background_env_path")"
wait_for_absence "$background_merged_env"

assert_contains 'INFISICAL_SECRET=from-infisical' "$background_env_snapshot"
assert_contains 'LOCAL_ONLY=from-env-file' "$background_env_snapshot"
assert_contains 'SUPABASE_URL=http://127.0.0.1:54321' "$background_runtime_env"

if [[ "$(<"$background_env_mode")" != "600" ]]; then
  echo "error: background merged env file permissions were not 600" >&2
  cat "$background_env_mode" >&2
  exit 1
fi

wait_for_path "$background_log"

echo "ok: supabase local functions uses secure temp env handling and reaches serve without RUNNER"
