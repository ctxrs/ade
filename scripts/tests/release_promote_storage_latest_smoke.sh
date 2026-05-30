#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

tmp="$(mktemp -d /tmp/ctx-release-promote-smoke.XXXXXX)"
server_pid=""

cleanup() {
  if [[ -n "$server_pid" ]]; then
    kill "$server_pid" >/dev/null 2>&1 || true
    wait "$server_pid" >/dev/null 2>&1 || true
  fi
  rm -rf "$tmp"
}
trap cleanup EXIT

port_file="$tmp/port.txt"
bucket="test-bucket"
version="0.4.99"
version_json="$tmp/${version}.json"
version_tauri_json="$tmp/${version}-tauri.json"
latest_json="$tmp/latest.json"
latest_tauri_json="$tmp/latest-tauri.json"
storage_override_channel="stable-dry-run-promote"
verified_manifest="$tmp/verified-version.json"
verified_manifest_sig="$tmp/verified-version.json.sig"
verified_tauri_manifest="$tmp/verified-version-tauri.json"
updater_pubkey="$tmp/updater_pubkey.txt"

cat >"$version_json" <<'JSON'
{
  "channel": "stable",
  "latest_version": "0.4.99",
  "published_at": "2026-03-10T00:00:00Z",
  "source_commit": "test-0.4.99",
  "platforms": {
    "macos-arm64": {
      "dmg": {
        "url_path": "/download/stable/0.4.99/ctx_0.4.99_macos-arm64.dmg",
        "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
      },
      "daemon": {
        "url_path": "/download/stable/0.4.99/ctx_0.4.99_macos-arm64",
        "sha256": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
      }
    }
  }
}
JSON
cp "$version_json" "$verified_manifest"
node - "$verified_manifest" "$verified_manifest_sig" "$updater_pubkey" <<'NODE'
const crypto = require("node:crypto");
const fs = require("node:fs");
const path = require("node:path");

const manifestPath = process.argv[2];
const signaturePath = process.argv[3];
const pubkeyPath = process.argv[4];
const ED25519_SPKI_PREFIX = Buffer.from("302a300506032b6570032100", "hex");
const { publicKey, privateKey } = crypto.generateKeyPairSync("ed25519");
const publicDer = publicKey.export({ format: "der", type: "spki" });
if (!publicDer.subarray(0, ED25519_SPKI_PREFIX.length).equals(ED25519_SPKI_PREFIX)) {
  throw new Error("unexpected Ed25519 public key DER prefix");
}
const keyId = crypto.randomBytes(8);
const publicPayload = Buffer.concat([Buffer.from("Ed"), keyId, publicDer.subarray(-32)]);
fs.writeFileSync(
  pubkeyPath,
  `untrusted comment: minisign public key: ${keyId.toString("hex")}\n${publicPayload.toString("base64")}\n`,
);

const manifestBytes = fs.readFileSync(manifestPath);
const signatureBytes = crypto.sign(null, manifestBytes, privateKey);
const trustedComment = `timestamp:${Math.floor(Date.now() / 1000)}\tfile:${path.basename(manifestPath)}`;
const globalSignatureBytes = crypto.sign(
  null,
  Buffer.concat([signatureBytes, Buffer.from(trustedComment, "utf8")]),
  privateKey,
);
const signaturePayload = Buffer.concat([Buffer.from("Ed"), keyId, signatureBytes]);
const signatureText = [
  "untrusted comment: signature from minisign secret key",
  signaturePayload.toString("base64"),
  `trusted comment: ${trustedComment}`,
  globalSignatureBytes.toString("base64"),
  "",
].join("\n");
fs.writeFileSync(signaturePath, `${Buffer.from(signatureText, "utf8").toString("base64")}\n`);
NODE

cat >"$version_tauri_json" <<'JSON'
{
  "version": "0.4.99",
  "notes": "ctx 0.4.99",
  "pub_date": "2026-03-10T00:00:00Z",
  "source_commit": "test-0.4.99",
  "platforms": {
    "macos-arm64": {
      "url": "https://api.ctx.rs/functions/v1/download/stable/0.4.99/ctx_0.4.99_macos-arm64_updater.app.tar.gz",
      "signature": "signature"
    }
  }
}
JSON
cp "$version_tauri_json" "$verified_tauri_manifest"

cat >"$latest_json" <<'JSON'
{
  "channel": "stable",
  "latest_version": "0.4.98",
  "published_at": "2026-03-09T00:00:00Z",
  "source_commit": "test-0.4.98",
  "platforms": {}
}
JSON

cat >"$latest_tauri_json" <<'JSON'
{
  "version": "0.4.98",
  "notes": "ctx 0.4.98",
  "pub_date": "2026-03-09T00:00:00Z",
  "source_commit": "test-0.4.98",
  "platforms": {}
}
JSON

PORT_FILE="$port_file" TMP_DIR="$tmp" BUCKET="$bucket" node - <<'NODE' >"$tmp/server.log" 2>&1 &
const fs = require("node:fs");
const http = require("node:http");
const path = require("node:path");

const tmpDir = process.env.TMP_DIR;
const portFile = process.env.PORT_FILE;
const bucket = process.env.BUCKET;

function objectPathToFile(pathname) {
  const r2Prefix = `/${bucket}/`;
  const compatibilityPrefix = `/storage/v1/object/${bucket}/`;
  if (!pathname.startsWith(r2Prefix) && !pathname.startsWith(compatibilityPrefix)) return null;
  const objectPath = pathname.startsWith(r2Prefix)
    ? pathname.slice(r2Prefix.length)
    : pathname.slice(compatibilityPrefix.length);
  return path.join(tmpDir, objectPath.replaceAll("/", "__"));
}

const seedFiles = new Map([
  [`releases/stable/0.4.99.json`, path.join(tmpDir, "0.4.99.json")],
  [`releases/stable/0.4.99-tauri.json`, path.join(tmpDir, "0.4.99-tauri.json")],
  [`releases/stable/latest.json`, path.join(tmpDir, "latest.json")],
  [`releases/stable/latest-tauri.json`, path.join(tmpDir, "latest-tauri.json")],
  [`releases/stable-dry-run-promote/0.4.99.json`, path.join(tmpDir, "0.4.99.json")],
  [`releases/stable-dry-run-promote/0.4.99-tauri.json`, path.join(tmpDir, "0.4.99-tauri.json")],
  [`releases/stable-dry-run-promote/latest.json`, path.join(tmpDir, "latest.json")],
  [`releases/stable-dry-run-promote/latest-tauri.json`, path.join(tmpDir, "latest-tauri.json")],
]);

for (const [objectPath, src] of seedFiles) {
  fs.copyFileSync(src, path.join(tmpDir, objectPath.replaceAll("/", "__")));
}

const server = http.createServer((req, res) => {
  const url = new URL(req.url, "http://127.0.0.1");
  const filePath = objectPathToFile(url.pathname);
  if (!filePath) {
    res.statusCode = 404;
    res.end("not found");
    return;
  }

  if (req.method === "GET") {
    if (!fs.existsSync(filePath)) {
      res.statusCode = 404;
      res.end("missing");
      return;
    }
    res.statusCode = 200;
    res.end(fs.readFileSync(filePath));
    return;
  }

  if (req.method === "POST" || req.method === "PUT") {
    const upsertHeader = String(req.headers["x-upsert"] || "").trim().toLowerCase();
    if ((upsertHeader === "false" || req.headers["if-none-match"] === "*") && fs.existsSync(filePath)) {
      res.statusCode = 409;
      res.end("exists");
      return;
    }
    const chunks = [];
    req.on("data", (chunk) => chunks.push(chunk));
    req.on("end", () => {
      fs.writeFileSync(filePath, Buffer.concat(chunks));
      res.statusCode = 200;
      res.end("ok");
    });
    return;
  }

  if (req.method === "DELETE") {
    fs.rmSync(filePath, { force: true });
    res.statusCode = 200;
    res.end("deleted");
    return;
  }

  res.statusCode = 405;
  res.end("method not allowed");
});

server.listen(0, "127.0.0.1", () => {
  fs.writeFileSync(portFile, String(server.address().port), "utf8");
});
NODE
server_pid=$!

for _ in $(seq 1 50); do
  if [[ -s "$port_file" ]]; then
    break
  fi
  sleep 0.1
done

if [[ ! -s "$port_file" ]]; then
  echo "error: test server did not report a port" >&2
  exit 1
fi

port="$(<"$port_file")"

RELEASE_R2_ENDPOINT="http://127.0.0.1:${port}" \
RELEASE_R2_ACCESS_KEY_ID="test-access-key" \
RELEASE_R2_SECRET_ACCESS_KEY="test-service-secret" \
RELEASE_STORAGE_BUCKET="$bucket" \
RELEASE_CHANNEL="stable" \
RELEASE_STORAGE_CHANNEL="$storage_override_channel" \
RELEASE_SOURCE_COMMIT="test-${version}" \
RELEASE_VERSION="$version" \
RELEASE_VERIFIED_MANIFEST_PATH="$verified_manifest" \
RELEASE_VERIFIED_MANIFEST_SIG_PATH="$verified_manifest_sig" \
RELEASE_VERIFIED_TAURI_MANIFEST_PATH="$verified_tauri_manifest" \
CTX_RELEASE_MANIFEST_PUBKEY_FILE="$updater_pubkey" \
RELEASE_STORAGE_PROVIDER="r2" \
./scripts/release_promote_storage_latest.sh >/dev/null

node - <<'NODE' "$tmp" "$storage_override_channel"
const fs = require("node:fs");
const path = require("node:path");

const tmpDir = process.argv[2];
const storageChannel = process.argv[3];
const versionLatest = fs.readFileSync(path.join(tmpDir, `releases__${storageChannel}__latest.json`), "utf8");
const versionLatestSig = fs.readFileSync(path.join(tmpDir, `releases__${storageChannel}__latest.json.sig`), "utf8");
const versionLatestTauri = fs.readFileSync(path.join(tmpDir, `releases__${storageChannel}__latest-tauri.json`), "utf8");
const expectedLatest = fs.readFileSync(path.join(tmpDir, `releases__${storageChannel}__0.4.99.json`), "utf8");
const expectedLatestTauri = fs.readFileSync(path.join(tmpDir, `releases__${storageChannel}__0.4.99-tauri.json`), "utf8");

if (versionLatest !== expectedLatest) {
  throw new Error("latest.json was not promoted from the version manifest");
}
const expectedLatestSig = fs.readFileSync(path.join(tmpDir, "verified-version.json.sig"), "utf8");
if (versionLatestSig !== expectedLatestSig) {
  throw new Error("latest.json.sig was not promoted from the verified version manifest signature");
}
if (versionLatestTauri !== expectedLatestTauri) {
  throw new Error("latest-tauri.json was not promoted from the version tauri manifest");
}
if (fs.existsSync(path.join(tmpDir, `releases__${storageChannel}__latest.manifest.lock`))) {
  throw new Error("manifest lock was not released");
}
console.log("ok: release promote latest smoke passed");
NODE
