#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if ! command -v supabase >/dev/null 2>&1; then
  echo "error: supabase CLI not found. Install: https://supabase.com/docs/guides/cli" >&2
  exit 1
fi

BUCKET="${SUPABASE_STORAGE_BUCKET:-releases}"

status_json="$(supabase status --output json)"
api_url="$(node -e 'const j=JSON.parse(process.argv[1]); console.log(j.api_url);' "$status_json")"
service_role_key="$(node -e 'const j=JSON.parse(process.argv[1]); console.log(j.service_role_key);' "$status_json")"

echo "Ensuring Storage bucket '$BUCKET' exists and is public..."
curl -fsS -X POST "$api_url/storage/v1/bucket" \
  -H "authorization: Bearer $service_role_key" \
  -H "content-type: application/json" \
  --data-binary "{\"id\":\"$BUCKET\",\"name\":\"$BUCKET\",\"public\":true}" >/dev/null || true

echo
echo "Local Supabase:"
echo "- API:       $api_url"
echo "- Studio:    http://127.0.0.1:54323"
echo "- Functions: $api_url/functions/v1"
echo "- Bucket:    $BUCKET"
echo
echo "Example (after you upload an object):"
echo "  curl -I \"$api_url/functions/v1/releases/stable/latest.json\""

