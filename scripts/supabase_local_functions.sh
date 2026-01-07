#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

MODE="foreground"
if [ "${1:-}" = "--background" ]; then
  MODE="background"
  shift
fi

if ! command -v supabase >/dev/null 2>&1; then
  echo "warning: supabase CLI not found; skipping local functions." >&2
  exit 0
fi

ENV_FILE="${SUPABASE_FUNCTIONS_ENV:-$ROOT/supabase/.env}"
ENV_FILE_ARG=()
if [ -f "$ENV_FILE" ]; then
  ENV_FILE_ARG=(--env-file "$ENV_FILE")
fi

status_json="$(supabase status --output json 2>/dev/null || true)"
if [ -z "$status_json" ] && [ "${#ENV_FILE_ARG[@]}" -eq 0 ]; then
  echo "warning: local Supabase not running; skipping functions." >&2
  exit 0
fi

supabase_url=""
service_role_key=""
anon_key=""
if [ -n "$status_json" ]; then
  supabase_url="$(node -e 'const j=JSON.parse(process.argv[1]); console.log(j.api_url ?? j.API_URL ?? "");' "$status_json")"
  service_role_key="$(node -e 'const j=JSON.parse(process.argv[1]); console.log(j.service_role_key ?? j.SERVICE_ROLE_KEY ?? "");' "$status_json")"
  anon_key="$(node -e 'const j=JSON.parse(process.argv[1]); console.log(j.anon_key ?? j.ANON_KEY ?? "");' "$status_json")"
fi

ENV_VARS=()
if [ -n "$supabase_url" ] && [ -n "$service_role_key" ]; then
  ENV_VARS+=("SUPABASE_URL=$supabase_url" "SUPABASE_SERVICE_ROLE_KEY=$service_role_key")
  if [ -n "$anon_key" ]; then
    ENV_VARS+=("SUPABASE_ANON_KEY=$anon_key")
  fi
elif [ "${#ENV_FILE_ARG[@]}" -eq 0 ]; then
  echo "warning: Supabase status missing keys; skipping functions." >&2
  exit 0
fi

use_infisical="${SUPABASE_FUNCTIONS_USE_INFISICAL:-1}"
MERGED_ENV="${SUPABASE_FUNCTIONS_MERGED_ENV:-/tmp/ctx-supabase-functions.env}"
mkdir -p "$(dirname "$MERGED_ENV")"
printf '' >"$MERGED_ENV"

if [ "$use_infisical" = "1" ]; then
  if ! command -v infisical >/dev/null 2>&1; then
    echo "warning: infisical CLI not found; running without Infisical." >&2
  else
    infisical_env="${INFISICAL_ENV:-dev}"
    infisical_path="${INFISICAL_PATH:-/}"
    infisical_project_dir="${INFISICAL_PROJECT_DIR:-$ROOT/core}"
    infisical_env_file="$(mktemp /tmp/ctx-infisical.env.XXXXXX)"
    (
      cd "$infisical_project_dir"
      infisical export --env "$infisical_env" --path "$infisical_path" --format dotenv --output-file "$infisical_env_file"
    )
    cat "$infisical_env_file" >>"$MERGED_ENV"
    rm -f "$infisical_env_file"
    printf '\n' >>"$MERGED_ENV"
  fi
fi

if [ -f "$ENV_FILE" ]; then
  cat "$ENV_FILE" >>"$MERGED_ENV"
  printf '\n' >>"$MERGED_ENV"
fi

for entry in "${ENV_VARS[@]}"; do
  printf '%s\n' "$entry" >>"$MERGED_ENV"
done

stripe_listen="${STRIPE_LISTEN:-1}"
if [ "$stripe_listen" = "1" ]; then
  stripe_secret="$(rg -m1 '^STRIPE_WEBHOOK_SECRET=' "$MERGED_ENV" | cut -d= -f2- || true)"
  if [ -z "$stripe_secret" ]; then
    if command -v stripe >/dev/null 2>&1; then
      stripe_forward_url="${STRIPE_WEBHOOK_FORWARD_URL:-http://127.0.0.1:54321/functions/v1/stripe-webhook}"
      stripe_log="${STRIPE_LISTEN_LOG:-/tmp/ctx-stripe-listen.log}"
      stripe_pid_file="${STRIPE_LISTEN_PID_FILE:-/tmp/ctx-stripe-listen.pid}"
      stripe_api_key="$(rg -m1 '^STRIPE_SECRET_KEY=' "$MERGED_ENV" | cut -d= -f2- || true)"

      if [ -f "$stripe_pid_file" ]; then
        old_pid="$(cat "$stripe_pid_file" 2>/dev/null || true)"
        if [ -n "$old_pid" ] && kill -0 "$old_pid" 2>/dev/null; then
          :
        else
          rm -f "$stripe_pid_file"
        fi
      fi

      if [ ! -f "$stripe_pid_file" ]; then
        : >"$stripe_log"
        if [ -n "$stripe_api_key" ]; then
          STRIPE_API_KEY="$stripe_api_key" nohup stripe listen --forward-to "$stripe_forward_url" >"$stripe_log" 2>&1 &
        else
          nohup stripe listen --forward-to "$stripe_forward_url" >"$stripe_log" 2>&1 &
        fi
        echo "$!" >"$stripe_pid_file"
        sleep 0.2
      fi

      stripe_secret=""
      for _ in $(seq 1 40); do
        stripe_secret="$(rg -o 'whsec_[A-Za-z0-9_]+' "$stripe_log" | tail -n 1 || true)"
        if [ -n "$stripe_secret" ]; then
          printf 'STRIPE_WEBHOOK_SECRET=%s\n' "$stripe_secret" >>"$MERGED_ENV"
          break
        fi
        sleep 0.25
      done
      if [ -z "$stripe_secret" ]; then
        echo "warning: stripe listen did not output a webhook secret; entitlements may not update." >&2
      fi
    else
      echo "warning: stripe CLI not found; entitlements require webhook forwarding." >&2
    fi
  fi
fi

CMD=(supabase functions serve --env-file "$MERGED_ENV" --no-verify-jwt)

if [ "$MODE" = "background" ]; then
  LOG_PATH="${SUPABASE_FUNCTIONS_LOG:-/tmp/ctx-supabase-functions.log}"
  nohup env "${ENV_VARS[@]}" "${RUNNER[@]}" "${CMD[@]}" >"$LOG_PATH" 2>&1 &
  echo "Supabase functions running (pid $!). Logs: $LOG_PATH"
  exit 0
fi

env "${ENV_VARS[@]}" "${RUNNER[@]}" "${CMD[@]}"
