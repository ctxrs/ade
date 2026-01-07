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

tailscale_ip=""
if command -v tailscale >/dev/null 2>&1; then
  tailscale_ip="$(tailscale ip -4 2>/dev/null | head -n 1 || true)"
fi

if [ -n "$tailscale_ip" ]; then
  api_host="$(node -e 'const u=new URL(process.argv[1]); console.log(u.hostname);' "$api_url")"
  if [ "$api_host" = "127.0.0.1" ] || [ "$api_host" = "localhost" ] || [ "$api_host" = "::1" ]; then
    api_url="$(node -e 'const u=new URL(process.argv[1]); u.hostname=process.argv[2]; console.log(u.toString());' "$api_url" "$tailscale_ip")"
  fi
fi

printf 'VITE_SUPABASE_URL=%s VITE_SUPABASE_ANON_KEY=%s' "$api_url" "$anon_key"
