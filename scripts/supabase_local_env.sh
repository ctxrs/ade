#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if ! command -v supabase >/dev/null 2>&1; then
  exit 0
fi

status_json="$(supabase status --output json 2>/dev/null || true)"
if [ -z "$status_json" ]; then
  exit 0
fi

api_url="$(node -e 'const j=JSON.parse(process.argv[1]); console.log(j.api_url ?? j.API_URL ?? "");' "$status_json")"
anon_key="$(node -e 'const j=JSON.parse(process.argv[1]); console.log(j.anon_key ?? j.ANON_KEY ?? "");' "$status_json")"

if [ -z "$api_url" ] || [ -z "$anon_key" ]; then
  exit 0
fi

printf 'VITE_SUPABASE_URL=%s VITE_SUPABASE_ANON_KEY=%s' "$api_url" "$anon_key"
