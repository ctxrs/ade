#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$ROOT/scripts/supabase_local_common.sh"

supabase_warn_missing_cli
supabase_prepare_local_project

status_json="$(supabase status --output json 2>/dev/null || true)"
if [ -n "$status_json" ]; then
  api_url="$(node -e 'const j=JSON.parse(process.argv[1]); console.log(j.api_url ?? j.API_URL ?? "");' "$status_json")"
else
  api_url=""
fi

if [ -n "$api_url" ]; then
  exit 0
fi

echo "Starting local Supabase..."
supabase start

if [ "${SUPABASE_RESET:-}" = "1" ]; then
  echo "Applying migrations (db reset)..."
  supabase db reset --no-seed
fi
