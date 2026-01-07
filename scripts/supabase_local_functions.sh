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
if [ -f "$ENV_FILE" ]; then
  CMD=(supabase functions serve --env-file "$ENV_FILE" --no-verify-jwt)
  ENV_VARS=()
else
  status_json="$(supabase status --output json 2>/dev/null || true)"
  if [ -z "$status_json" ]; then
    echo "warning: local Supabase not running; skipping functions." >&2
    exit 0
  fi

  supabase_url="$(node -e 'const j=JSON.parse(process.argv[1]); console.log(j.api_url ?? j.API_URL ?? "");' "$status_json")"
  service_role_key="$(node -e 'const j=JSON.parse(process.argv[1]); console.log(j.service_role_key ?? j.SERVICE_ROLE_KEY ?? "");' "$status_json")"
  anon_key="$(node -e 'const j=JSON.parse(process.argv[1]); console.log(j.anon_key ?? j.ANON_KEY ?? "");' "$status_json")"

  if [ -z "$supabase_url" ] || [ -z "$service_role_key" ]; then
    echo "warning: Supabase status missing keys; skipping functions." >&2
    exit 0
  fi

  CMD=(supabase functions serve --no-verify-jwt)
  ENV_VARS=(
    "SUPABASE_URL=$supabase_url"
    "SUPABASE_SERVICE_ROLE_KEY=$service_role_key"
    "SUPABASE_ANON_KEY=$anon_key"
  )
fi

RUNNER=()
if [ "${SUPABASE_FUNCTIONS_USE_INFISICAL:-}" = "1" ]; then
  if ! command -v infisical >/dev/null 2>&1; then
    echo "warning: infisical CLI not found; running without Infisical." >&2
  else
    infisical_env="${INFISICAL_ENV:-dev}"
    infisical_path="${INFISICAL_PATH:-/}"
    infisical_project_dir="${INFISICAL_PROJECT_DIR:-$ROOT/core}"
    RUNNER=(infisical run --project-config-dir "$infisical_project_dir" --env "$infisical_env" --path "$infisical_path" --)
  fi
fi

if [ "$MODE" = "background" ]; then
  LOG_PATH="${SUPABASE_FUNCTIONS_LOG:-/tmp/ctx-supabase-functions.log}"
  nohup env "${ENV_VARS[@]}" "${RUNNER[@]}" "${CMD[@]}" >"$LOG_PATH" 2>&1 &
  echo "Supabase functions running (pid $!). Logs: $LOG_PATH"
  exit 0
fi

env "${ENV_VARS[@]}" "${RUNNER[@]}" "${CMD[@]}"
