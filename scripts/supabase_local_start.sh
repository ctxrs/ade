#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if ! command -v supabase >/dev/null 2>&1; then
  echo "error: supabase CLI not found. Install: https://supabase.com/docs/guides/cli" >&2
  exit 1
fi

echo "Starting local Supabase..."
supabase start

echo "Applying migrations..."
supabase db reset

echo "Done. Run ./scripts/supabase_local_seed.sh to create the releases bucket."

