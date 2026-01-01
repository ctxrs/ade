#!/usr/bin/env bash
set -euo pipefail

if ! command -v pnpm >/dev/null; then
  echo "pnpm required" >&2
  exit 1
fi

URL=${1:-http://localhost:19006}
OUT=${2:-/tmp/ctx-mobile-$(date +%Y%m%d-%H%M%S).png}

echo "Capturing mobile screenshot from $URL -> $OUT"
pnpm -C core/apps/web exec playwright screenshot --device="iPhone 13 Pro" "$URL" "$OUT"
echo "Saved screenshot to $OUT"
