#!/usr/bin/env node
"use strict";

const crypto = require("node:crypto");
const fs = require("node:fs");
const path = require("node:path");

const MANIFEST_PATH = String(process.argv[2] || "").trim();
const PLATFORM = String(process.argv[3] || "").trim();
const ARTIFACT_PATH = String(process.argv[4] || "").trim();
const ENV_PUBKEY = String(process.env.CTX_DESKTOP_UPDATER_PUBKEY || "").trim();
const DEFAULT_PUBKEY_PATH = path.resolve(
  __dirname,
  "..",
  "apps",
  "desktop",
  "src-tauri",
  "config",
  "updater_pubkey.txt",
);
const ENV_PUBKEY_FILE = String(process.env.CTX_DESKTOP_UPDATER_PUBKEY_FILE || "").trim();

const ED25519_SPKI_PREFIX = Buffer.from("302a300506032b6570032100", "hex");
const TRUSTED_COMMENT_PREFIX = "trusted comment: ";

const fail = (message) => {
  console.error(`error: ${message}`);
  process.exit(1);
};

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

  return bytes;
};

const decodeBase64Utf8Strict = (label, encodedRaw) => {
  const bytes = decodeBase64Strict(label, encodedRaw);
  const text = bytes.toString("utf8");
  if (!Buffer.from(text, "utf8").equals(bytes)) {
    fail(`${label}: decoded payload is not valid utf-8 text`);
  }
  return text;
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

const normalizeUpdaterPubkeyText = (raw, label) => {
  const value = String(raw || "").trim();
  if (!value) return null;
  const plain = normalizeMinisignPubkeyText(value);
  if (plain) return plain;
  const compact = value.replace(/\s+/g, "");
  const decoded = decodeBase64Utf8Strict(label, compact);
  const normalizedDecoded = normalizeMinisignPubkeyText(decoded);
  if (!normalizedDecoded) {
    fail(`${label} is not a valid minisign public key (plain or base64)`);
  }
  return normalizedDecoded;
};

const resolveUpdaterPubkeyText = () => {
  const envValue = normalizeUpdaterPubkeyText(ENV_PUBKEY, "CTX_DESKTOP_UPDATER_PUBKEY");
  if (envValue) return envValue;

  const pubkeyPath = ENV_PUBKEY_FILE || DEFAULT_PUBKEY_PATH;
  let fileRaw = "";
  try {
    fileRaw = fs.readFileSync(pubkeyPath, "utf8");
  } catch (err) {
    fail(
      `failed to read updater pubkey file '${pubkeyPath}': ${String(err)}`
    );
  }
  const fileValue = normalizeUpdaterPubkeyText(fileRaw, `updater pubkey file '${pubkeyPath}'`);
  if (!fileValue) {
    fail(`updater pubkey file '${pubkeyPath}' is empty`);
  }
  return fileValue;
};

const splitMeaningfulLines = (raw) =>
  String(raw || "")
    .replace(/\r\n/g, "\n")
    .split("\n")
    .map((line) => line.trim())
    .filter(Boolean);

const expectAlgorithm = (label, value) => {
  const legacy = value[0] === 0x45 && value[1] === 0x64;
  const prehashed = value[0] === 0x45 && value[1] === 0x44;
  if (!legacy && !prehashed) {
    fail(`${label}: unsupported minisign algorithm bytes ${value.toString("hex")}`);
  }
  return { isPrehashed: prehashed };
};

const parseMinisignPublicKey = (label, rawText) => {
  const lines = splitMeaningfulLines(rawText);
  if (lines.length !== 2) {
    fail(`${label}: expected exactly 2 non-empty lines`);
  }
  if (!lines[0].startsWith("untrusted comment: minisign public key:")) {
    fail(`${label}: missing minisign public key header`);
  }
  const payload = decodeBase64Strict(`${label} payload`, lines[1]);
  if (payload.length !== 42) {
    fail(`${label}: decoded payload must be 42 bytes, got ${payload.length}`);
  }
  expectAlgorithm(`${label} payload`, payload.subarray(0, 2));
  return {
    keyId: payload.subarray(2, 10),
    publicKeyBytes: payload.subarray(10, 42),
  };
};

const parseMinisignSignature = (label, rawText) => {
  const lines = splitMeaningfulLines(rawText);
  if (lines.length !== 4) {
    fail(`${label}: expected exactly 4 non-empty lines`);
  }
  if (!lines[0].startsWith("untrusted comment: signature from")) {
    fail(`${label}: missing minisign signature header`);
  }
  if (!lines[2].startsWith(TRUSTED_COMMENT_PREFIX)) {
    fail(`${label}: missing trusted comment line`);
  }

  const signaturePayload = decodeBase64Strict(`${label} signature payload`, lines[1]);
  if (signaturePayload.length !== 74) {
    fail(`${label}: signature payload must be 74 bytes, got ${signaturePayload.length}`);
  }
  const globalSignaturePayload = decodeBase64Strict(`${label} global signature payload`, lines[3]);
  if (globalSignaturePayload.length !== 64) {
    fail(
      `${label}: global signature payload must be 64 bytes, got ${globalSignaturePayload.length}`,
    );
  }

  const { isPrehashed } = expectAlgorithm(
    `${label} signature payload`,
    signaturePayload.subarray(0, 2),
  );
  return {
    keyId: signaturePayload.subarray(2, 10),
    signatureBytes: signaturePayload.subarray(10, 74),
    globalSignatureBytes: globalSignaturePayload,
    trustedComment: lines[2].slice(TRUSTED_COMMENT_PREFIX.length),
    isPrehashed,
  };
};

const createEd25519PublicKey = (rawPublicKey) =>
  crypto.createPublicKey({
    key: Buffer.concat([ED25519_SPKI_PREFIX, rawPublicKey]),
    format: "der",
    type: "spki",
  });

const verifyEd25519 = (label, data, publicKey, signature) => {
  const ok = crypto.verify(null, data, publicKey, signature);
  if (!ok) {
    fail(`${label}: signature verification failed`);
  }
};

const verifyMinisignArtifact = (label, artifactBytes, signatureText, pubkeyText) => {
  const publicKey = parseMinisignPublicKey(`${label} public key`, pubkeyText);
  const signature = parseMinisignSignature(`${label} signature`, signatureText);

  if (!publicKey.keyId.equals(signature.keyId)) {
    fail(`${label}: signature key id does not match updater public key`);
  }

  const keyObject = createEd25519PublicKey(publicKey.publicKeyBytes);
  const signedBytes = signature.isPrehashed
    ? crypto.createHash("blake2b512").update(artifactBytes).digest()
    : artifactBytes;
  verifyEd25519(`${label} artifact`, signedBytes, keyObject, signature.signatureBytes);

  const globalBytes = Buffer.concat([
    signature.signatureBytes,
    Buffer.from(signature.trustedComment, "utf8"),
  ]);
  verifyEd25519(`${label} trusted comment`, globalBytes, keyObject, signature.globalSignatureBytes);
};

if (!MANIFEST_PATH || !PLATFORM || !ARTIFACT_PATH) {
  fail(
    "usage: node core/scripts/updater_contract_validate.cjs <latest-tauri.json-path> <platform> <artifact-path>",
  );
}

const pubkeyText = resolveUpdaterPubkeyText();

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

const entry = manifest.platforms[PLATFORM];
if (!entry || typeof entry !== "object" || Array.isArray(entry)) {
  fail(`platform '${PLATFORM}' entry must be an object`);
}

const urlRaw = String(entry.url || "").trim();
if (!urlRaw) {
  fail(`platform '${PLATFORM}' url is missing`);
}

let expectedArtifactName = "";
try {
  expectedArtifactName = path.posix.basename(new URL(urlRaw).pathname);
} catch (err) {
  fail(`platform '${PLATFORM}' url is invalid: ${String(err)}`);
}
if (!expectedArtifactName) {
  fail(`platform '${PLATFORM}' url does not include an artifact path`);
}

if (path.basename(ARTIFACT_PATH) !== expectedArtifactName) {
  fail(
    `platform '${PLATFORM}' artifact basename mismatch: expected '${expectedArtifactName}', got '${path.basename(ARTIFACT_PATH)}'`,
  );
}

const signatureField = String(entry.signature || "").trim();
if (!signatureField) {
  fail(`platform '${PLATFORM}' signature is missing`);
}

let artifactBytes;
try {
  artifactBytes = fs.readFileSync(ARTIFACT_PATH);
} catch (err) {
  fail(`failed to read artifact '${ARTIFACT_PATH}': ${String(err)}`);
}

const signatureText = decodeBase64Utf8Strict(`platform '${PLATFORM}' signature`, signatureField);
verifyMinisignArtifact(`platform '${PLATFORM}'`, artifactBytes, signatureText, pubkeyText);

console.log(
  `ok: updater artifact verified for platform '${PLATFORM}' (${expectedArtifactName})`,
);
