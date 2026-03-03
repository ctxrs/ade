#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PARSER="$ROOT/scripts/lib/read_tauri_signature.awk"

tmp="$(mktemp -d /tmp/ctx-release-signature-parser.XXXXXX)"
trap 'rm -rf "$tmp"' EXIT

sig_a="RWS6vR7Y3Ih6I4h0n8p9r1v6u3A8k3N9i4L3H5M6N8P1Q2R3S4T5U6V7W8X9Y0ZaB1cD2eF3gH4iJ5kL6mN7oP8qR9sT0uV1wX2yZ3"
sig_b="RWQn2L4k6m8p0s2v4x6z8B0D2F4H6J8L0N2P4R6T8V0X2Z4b6d8f0h2j4l6n8p0r2t4v6x8z0B2D4F6H8J0L2N4P6R8T0V2X4Z6b8d0"
sig_c="dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIHRhdXJpIHNlY3JldCBrZXkKUlVTY2U5Zk5jejlRRFlrOEp5NDdPMmI5dkJlY1ZuVC9EQUMxdVNCTEhlYmxXZlF0K0R1SmZCVyswRUREMU4yemRJQitZVEszK1ZCeTMwYzRYWnRLaEdMQWJFYmgydHRxb3dFPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxNzcyNTE0NDM0CWZpbGU6Y3R4LmFwcC50YXIuZ3oKZ3lib3g4c25zQUd5L2ZmUHFqaFdQYlZMNUJtR3hBSi9Rb2N1NXQvRDhBRmgyRlhFd2d5UmhpZER2R2w5R0pqSHh3RHhiakJpOGxFTk9pZDIzRHRsQVE9PQo="

cat >"$tmp/signature-first.txt" <<EOF
Signature:
${sig_a}
Make sure to include this into the signature field of your update server.
EOF

out="$(awk -f "$PARSER" "$tmp/signature-first.txt" | tr -d '\r\n')"
if [[ "$out" != "$sig_a" ]]; then
  echo "error: parser failed signature-first format" >&2
  echo "expected: $sig_a" >&2
  echo "actual:   $out" >&2
  exit 1
fi

cat >"$tmp/signature-inline.txt" <<EOF
info: signing complete
Signature: ${sig_b}
Make sure to include this into the signature field of your update server.
EOF

out="$(awk -f "$PARSER" "$tmp/signature-inline.txt" | tr -d '\r\n')"
if [[ "$out" != "$sig_b" ]]; then
  echo "error: parser failed signature-inline format" >&2
  echo "expected: $sig_b" >&2
  echo "actual:   $out" >&2
  exit 1
fi

cat >"$tmp/public-signature.txt" <<EOF
Your file was signed successfully, You can find the signature here:
/tmp/ctx.app.tar.gz.sig

Public signature:
${sig_c}

Make sure to include this into the signature field of your update server.
EOF

out="$(awk -f "$PARSER" "$tmp/public-signature.txt" | tr -d '\r\n')"
if [[ "$out" != "$sig_c" ]]; then
  echo "error: parser failed public-signature format" >&2
  echo "expected: $sig_c" >&2
  echo "actual:   $out" >&2
  exit 1
fi

cat >"$tmp/no-signature.txt" <<'EOF'
info: signing failed
Make sure to include this into the signature field of your update server.
EOF

out="$(awk -f "$PARSER" "$tmp/no-signature.txt" | tr -d '\r\n')"
if [[ -n "$out" ]]; then
  echo "error: parser should return empty when no signature exists" >&2
  echo "actual: $out" >&2
  exit 1
fi

echo "ok: tauri signature parser smoke passed"
