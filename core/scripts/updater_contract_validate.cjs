#!/usr/bin/env node
"use strict";

const fs = require("node:fs");

const MANIFEST_PATH = String(process.argv[2] || "").trim();
const ENV_PUBKEY = String(process.env.CTX_DESKTOP_UPDATER_PUBKEY || "").trim();

const fail = (message) => {
  console.error(`error: ${message}`);
  process.exit(1);
};

const toBase64Canonical = (value) => Buffer.from(value, "utf8").toString("base64");

const decodeBase64Strict = (label, encodedRaw) => {
  const encoded = String(encodedRaw || "").trim();
  if (!encoded) fail(`${label}: value is empty`);
  if (/\s/.test(encoded)) fail(`${label}: base64 must not contain whitespace`);
  if (!/^[A-Za-z0-9+/=]+$/.test(encoded)) fail(`${label}: invalid base64 characters`);
  if (encoded.length % 4 !== 0) fail(`${label}: base64 length is not a multiple of 4`);

  let bytes;
  try {
    bytes = Buffer.from(encoded, "base64");
  } catch (err) {
    fail(`${label}: base64 decode failed: ${String(err)}`);
  }

  const roundTrip = bytes.toString("base64");
  const stripPadding = (value) => value.replace(/=+$/g, "");
  if (stripPadding(roundTrip) !== stripPadding(encoded)) {
    fail(`${label}: base64 round-trip mismatch`);
  }

  return bytes.toString("utf8");
};

const normalizeMinisignPubkeyText = (raw) => {
  const normalized = String(raw || "").replace(/\r\n/g, "\n");
  const lines = normalized
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean);
  if (lines.length !== 2) return null;
  const [header, keyLine] = lines;
  if (!header.startsWith("untrusted comment: minisign public key:")) return null;
  if (!keyLine) return null;
  return `${header}\n${keyLine}\n`;
};

const normalizeUpdaterPubkey = (raw) => {
  const value = String(raw || "").trim();
  if (!value) return null;
  const plain = normalizeMinisignPubkeyText(value);
  if (plain) return toBase64Canonical(plain);
  const compact = value.replace(/\s+/g, "");
  const decoded = decodeBase64Strict("CTX_DESKTOP_UPDATER_PUBKEY", compact);
  const normalizedDecoded = normalizeMinisignPubkeyText(decoded);
  if (!normalizedDecoded) return null;
  return toBase64Canonical(normalizedDecoded);
};

const validateMinisignSignatureText = (label, decoded) => {
  const lines = String(decoded || "")
    .replace(/\r\n/g, "\n")
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean);
  if (lines.length < 4) {
    fail(`${label}: decoded signature is missing required lines`);
  }
  if (!lines[0].startsWith("untrusted comment: signature from")) {
    fail(`${label}: missing minisign signature header`);
  }
  if (!lines[1]) {
    fail(`${label}: missing minisign signature body`);
  }
  if (!lines[2].startsWith("trusted comment:")) {
    fail(`${label}: missing trusted comment line`);
  }
  if (!lines[3]) {
    fail(`${label}: missing trusted signature payload`);
  }
};

if (!MANIFEST_PATH) {
  fail("usage: node core/scripts/updater_contract_validate.cjs <latest-tauri.json-path>");
}

let manifestRaw = "";
try {
  manifestRaw = fs.readFileSync(MANIFEST_PATH, "utf8");
} catch (err) {
  fail(`failed to read manifest '${MANIFEST_PATH}': ${String(err)}`);
}

let manifest;
try {
  manifest = JSON.parse(manifestRaw);
} catch (err) {
  fail(`failed to parse manifest '${MANIFEST_PATH}' as json: ${String(err)}`);
}

if (!manifest || typeof manifest !== "object") {
  fail("manifest must be a json object");
}

if (!manifest.platforms || typeof manifest.platforms !== "object" || Array.isArray(manifest.platforms)) {
  fail("manifest.platforms must be an object");
}

const platformEntries = Object.entries(manifest.platforms);
if (platformEntries.length === 0) {
  fail("manifest.platforms is empty");
}

for (const [platform, entry] of platformEntries) {
  if (!entry || typeof entry !== "object" || Array.isArray(entry)) {
    fail(`platform '${platform}' entry must be an object`);
  }
  const signature = String(entry.signature || "").trim();
  if (!signature) {
    fail(`platform '${platform}' signature is missing`);
  }
  const decoded = decodeBase64Strict(`platform '${platform}' signature`, signature);
  validateMinisignSignatureText(`platform '${platform}' signature`, decoded);
}

if (ENV_PUBKEY) {
  const canonicalPubkey = normalizeUpdaterPubkey(ENV_PUBKEY);
  if (!canonicalPubkey) {
    fail("CTX_DESKTOP_UPDATER_PUBKEY is not a valid minisign public key (plain or base64)");
  }
  const decodedPubkey = decodeBase64Strict("CTX_DESKTOP_UPDATER_PUBKEY (canonical)", canonicalPubkey);
  if (!normalizeMinisignPubkeyText(decodedPubkey)) {
    fail("CTX_DESKTOP_UPDATER_PUBKEY canonical decode is not a minisign public key");
  }
}

console.log(`ok: updater contract validated for ${platformEntries.length} platform entries`);
