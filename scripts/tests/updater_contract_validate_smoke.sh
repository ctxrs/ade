#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$ROOT/core/scripts/updater_contract_validate.cjs"

tmp="$(mktemp -d /tmp/ctx-updater-contract.XXXXXX)"
trap 'rm -rf "$tmp"' EXIT

artifact_name="ctx_0.4.99_macos-arm64_updater.app.tar.gz"
artifact_path="$tmp/$artifact_name"
manifest_path="$tmp/latest-tauri.valid.json"
pubkey_txt="$tmp/updater.pub"
pubkey_b64_path="$tmp/updater.pub.b64"
signature_b64_path="$tmp/updater.signature.b64"

printf 'ctx updater smoke artifact\nversion=0.4.99\n' >"$artifact_path"

ARTIFACT_PATH="$artifact_path" \
PUBKEY_TXT="$pubkey_txt" \
PUBKEY_B64_PATH="$pubkey_b64_path" \
SIGNATURE_B64_PATH="$signature_b64_path" \
node - <<'NODE'
const crypto = require("node:crypto");
const fs = require("node:fs");
const path = require("node:path");

function toBase64UrlBuffer(value) {
  const normalized = String(value).replace(/-/g, "+").replace(/_/g, "/");
  const padded = normalized + "=".repeat((4 - (normalized.length % 4)) % 4);
  return Buffer.from(padded, "base64");
}

const artifactPath = process.env.ARTIFACT_PATH;
const artifactName = path.basename(artifactPath);
const pubkeyTxt = process.env.PUBKEY_TXT;
const pubkeyB64Path = process.env.PUBKEY_B64_PATH;
const signatureB64Path = process.env.SIGNATURE_B64_PATH;
const artifactBytes = fs.readFileSync(artifactPath);

const { publicKey, privateKey } = crypto.generateKeyPairSync("ed25519");
const publicJwk = publicKey.export({ format: "jwk" });
const rawPublicKey = toBase64UrlBuffer(publicJwk.x);
const keyId = crypto.randomBytes(8);

const publicKeyPayload = Buffer.concat([Buffer.from([0x45, 0x64]), keyId, rawPublicKey]);
const publicKeyText =
  "untrusted comment: minisign public key: 0D503F73CDD77B9C\n" +
  `${publicKeyPayload.toString("base64")}\n`;

const signatureBytes = crypto.sign(
  null,
  crypto.createHash("blake2b512").update(artifactBytes).digest(),
  privateKey,
);
const trustedComment = `trusted comment: timestamp:1772585616\tfile:${artifactName}`;
const signaturePayload = Buffer.concat([Buffer.from([0x45, 0x44]), keyId, signatureBytes]);
const globalSignature = crypto.sign(
  null,
  Buffer.concat([
    signatureBytes,
    Buffer.from(trustedComment.slice("trusted comment: ".length), "utf8"),
  ]),
  privateKey,
);
const signatureText =
  "untrusted comment: signature from minisign secret key\n" +
  `${signaturePayload.toString("base64")}\n` +
  `${trustedComment}\n` +
  `${globalSignature.toString("base64")}\n`;

fs.writeFileSync(pubkeyTxt, publicKeyText, "utf8");
fs.writeFileSync(pubkeyB64Path, Buffer.from(publicKeyText, "utf8").toString("base64"), "utf8");
fs.writeFileSync(signatureB64Path, Buffer.from(signatureText, "utf8").toString("base64"), "utf8");
NODE

pubkey_raw="$(<"$pubkey_txt")"
pubkey_b64="$(tr -d '\r\n' < "$pubkey_b64_path")"
signature_b64="$(tr -d '\r\n' < "$signature_b64_path")"

cat >"$manifest_path" <<EOF
{
  "version": "0.4.99",
  "notes": "ctx 0.4.99",
  "pub_date": "2026-03-04T00:00:00Z",
  "platforms": {
    "macos-arm64": {
      "url": "https://example.test/download/stable/0.4.99/${artifact_name}",
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

CTX_DESKTOP_UPDATER_PUBKEY="$pubkey_b64" node "$SCRIPT" "$manifest_path" "macos-arm64" "$artifact_path" >/dev/null
CTX_DESKTOP_UPDATER_PUBKEY="$pubkey_raw" node "$SCRIPT" "$manifest_path" "macos-arm64" "$artifact_path" >/dev/null

tampered_dir="$tmp/tampered"
mkdir -p "$tampered_dir"
tampered_artifact="$tampered_dir/$artifact_name"
printf 'tampered artifact bytes\n' >"$tampered_artifact"

expect_fail "missing-pubkey" env -u CTX_DESKTOP_UPDATER_PUBKEY node "$SCRIPT" "$manifest_path" "macos-arm64" "$artifact_path"
expect_fail "tampered-artifact" env CTX_DESKTOP_UPDATER_PUBKEY="$pubkey_b64" node "$SCRIPT" "$manifest_path" "macos-arm64" "$tampered_artifact"
expect_fail "bad-pubkey" env CTX_DESKTOP_UPDATER_PUBKEY="not-a-valid-minisign-key" node "$SCRIPT" "$manifest_path" "macos-arm64" "$artifact_path"

echo "ok: updater contract validator smoke passed"
