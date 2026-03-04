#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$ROOT/core/scripts/updater_contract_validate.cjs"

tmp="$(mktemp -d /tmp/ctx-updater-contract.XXXXXX)"
trap 'rm -rf "$tmp"' EXIT

encode_b64() {
  printf '%s' "$1" | base64 | tr -d '\r\n'
}

signature_raw=$'untrusted comment: signature from minisign secret key\nRUTestSignatureBody0123456789\ntrusted comment: timestamp:1772585616\tfile:ctx.app.tar.gz\nSignedPayload0123456789\n'
signature_b64="$(encode_b64 "$signature_raw")"

pubkey_raw=$'untrusted comment: minisign public key: 0D503F73CDD77B9C\nRWSce9fNcz9QDfv7dghgOH/dIA0Txkgk8rB86J5s6I15e+NkpWjU3CFs\n'

cat >"$tmp/latest-tauri.valid.json" <<EOF
{
  "version": "0.4.99",
  "notes": "ctx 0.4.99",
  "pub_date": "2026-03-04T00:00:00Z",
  "platforms": {
    "macos-arm64": {
      "url": "https://example.test/download/stable/0.4.99/ctx_0.4.99_macos-arm64_updater.app.tar.gz",
      "signature": "${signature_b64}"
    }
  }
}
EOF

expect_fail() {
  local name="$1"
  shift
  if "$@" >/tmp/ctx-updater-contract-${name}.out 2>&1; then
    echo "error: expected failure for ${name}" >&2
    cat /tmp/ctx-updater-contract-${name}.out >&2 || true
    exit 1
  fi
}

CTX_DESKTOP_UPDATER_PUBKEY="$pubkey_raw" node "$SCRIPT" "$tmp/latest-tauri.valid.json" >/dev/null

cat >"$tmp/latest-tauri.bad-signature.json" <<EOF
{
  "version": "0.4.99",
  "notes": "ctx 0.4.99",
  "pub_date": "2026-03-04T00:00:00Z",
  "platforms": {
    "macos-arm64": {
      "url": "https://example.test/download/stable/0.4.99/ctx_0.4.99_macos-arm64_updater.app.tar.gz",
      "signature": "abc def"
    }
  }
}
EOF

expect_fail "bad-signature" node "$SCRIPT" "$tmp/latest-tauri.bad-signature.json"

expect_fail "bad-pubkey" env CTX_DESKTOP_UPDATER_PUBKEY="not-a-valid-minisign-key" node "$SCRIPT" "$tmp/latest-tauri.valid.json"

echo "ok: updater contract validator smoke passed"
