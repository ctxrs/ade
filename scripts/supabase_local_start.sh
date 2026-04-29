#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$ROOT/scripts/supabase_local_common.sh"

supabase_require_cli
supabase_prepare_local_project

echo "Starting local Supabase..."
supabase start

echo "Applying migrations..."
supabase db reset --no-seed

echo "Done. Run ./scripts/supabase_local_seed.sh to create the releases bucket."
