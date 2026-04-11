#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

tmp="$(mktemp -d /tmp/ctx-allowlist-contract.XXXXXX)"
trap 'rm -rf "$tmp"' EXIT

functions_base="http://127.0.0.1:9/functions/v1" # guaranteed to fail quickly; we just test gating

run_verify() {
  local channel="$1"
  local out_file="$tmp/out_${channel}.log"
  set +e
  SUPABASE_FUNCTIONS_URL="$functions_base" RELEASE_CHANNEL="$channel" \
    ./scripts/release_verify_supabase.sh >"$out_file" 2>&1
  local code=$?
  set -e
  printf '%s' "$out_file"
  echo "$code" >"${out_file}.code"
  return 0
}

assert_contains() {
  local file="$1" needle="$2"
  if ! grep -q -- "$needle" "$file"; then
    echo "assertion failed: expected output to contain: $needle" >&2
    echo "--- output ---" >&2
    cat "$file" >&2 || true
    exit 1
  fi
}

assert_not_contains() {
  local file="$1" needle="$2"
  if grep -q -- "$needle" "$file"; then
    echo "assertion failed: expected output to NOT contain: $needle" >&2
    echo "--- output ---" >&2
    cat "$file" >&2 || true
    exit 1
  fi
}

echo "contract: invalid channel is rejected"
set +e
out_path="$(run_verify "bogus" )"
code="$(cat "${out_path}.code")"
set -e
if [[ $code -ne 2 ]]; then
  echo "expected exit code 2 for unsupported channel, got $code" >&2
  exit 1
fi
assert_contains "$out_path" "unsupported RELEASE_CHANNEL"

for ch in stable canary canary2 e2e; do
  echo "contract: valid channel '$ch' passes allowlist gate"
  out_path="$(run_verify "$ch")"
  code="$(cat "${out_path}.code")"
  # We only assert it did not fail on the allowlist; network will fail later
  assert_not_contains "$out_path" "unsupported RELEASE_CHANNEL"
  assert_contains "$out_path" "unable to fetch release manifests"
done

echo "ok: release channel allowlist contract holds (stable, canary, canary2, e2e)"
