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

MERGED_ENV="${SUPABASE_FUNCTIONS_MERGED_ENV:-/tmp/ctx-supabase-functions.env}"
mkdir -p "$(dirname "$MERGED_ENV")"
printf '' >"$MERGED_ENV"

if [ -f "$ENV_FILE" ]; then
  cat "$ENV_FILE" >>"$MERGED_ENV"
  printf '\n' >>"$MERGED_ENV"
fi

use_infisical="${SUPABASE_FUNCTIONS_USE_INFISICAL:-1}"
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

for entry in "${ENV_VARS[@]}"; do
  printf '%s\n' "$entry" >>"$MERGED_ENV"
done

CMD=(supabase functions serve --env-file "$MERGED_ENV" --no-verify-jwt)

if [ "$MODE" = "background" ]; then
  LOG_PATH="${SUPABASE_FUNCTIONS_LOG:-/tmp/ctx-supabase-functions.log}"
  nohup env "${ENV_VARS[@]}" "${RUNNER[@]}" "${CMD[@]}" >"$LOG_PATH" 2>&1 &
  echo "Supabase functions running (pid $!). Logs: $LOG_PATH"
  exit 0
fi

env "${ENV_VARS[@]}" "${RUNNER[@]}" "${CMD[@]}"
