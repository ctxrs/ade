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

dev_host=""
if command -v tailscale >/dev/null 2>&1; then
  dev_host="$(tailscale ip -4 2>/dev/null | head -n 1 || true)"
fi
if [ -z "$dev_host" ]; then
  dev_host="127.0.0.1"
fi

dev_port="${CTX_WEB_PORT:-5173}"
dev_origin="https://${dev_host}:${dev_port}"
https_hosts="${dev_host},localhost,127.0.0.1"

printf 'VITE_SUPABASE_URL=%s VITE_SUPABASE_ANON_KEY=%s CTX_SUPABASE_PROXY_TARGET=%s CTX_DEV_HTTPS_HOSTS=%s' \
  "$dev_origin" \
  "$anon_key" \
  "$api_url" \
  "$https_hosts"
