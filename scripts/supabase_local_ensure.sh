#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if ! command -v supabase >/dev/null 2>&1; then
  echo "warning: supabase CLI not found; skipping local Supabase start." >&2
  exit 0
fi

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
  supabase db reset
fi
