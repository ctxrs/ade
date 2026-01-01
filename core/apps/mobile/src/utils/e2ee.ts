import "react-native-get-random-values";
import { Buffer } from "buffer";
import nacl from "tweetnacl";
import { HKDF } from "@stablelib/hkdf";
import { SHA256 } from "@stablelib/sha256";
import { XChaCha20Poly1305 } from "@stablelib/xchacha20poly1305";

const HKDF_INFO = new TextEncoder().encode("ctx-mobile-e2ee-v1");

export type E2eeEnvelope = {
  device_id: string;
  seq: number;
  nonce: string;
  ciphertext: string;
};

export const deriveSharedKey = (deviceSecretKeyB64: string, daemonPublicKeyB64: string, deviceId: string) => {
  const deviceSecret = decodeBase64(deviceSecretKeyB64);
  const daemonPublic = decodeBase64(daemonPublicKeyB64);
  const shared = nacl.scalarMult(deviceSecret, daemonPublic);
  const salt = sha256Bytes(deviceId);
  const hkdf = new HKDF(SHA256, shared, salt, HKDF_INFO);
  const key = hkdf.expand(32);
  hkdf.clean();
  return key;
};

export const encryptPayload = (
  key: Uint8Array,
  deviceId: string,
  seq: number,
  plaintext: Uint8Array,
): E2eeEnvelope => {
  const nonce = randomBytes(24);
  const aad = buildAad(deviceId, seq);
  const cipher = new XChaCha20Poly1305(key);
  const ciphertext = cipher.seal(nonce, plaintext, aad);
  return {
    device_id: deviceId,
    seq,
    nonce: encodeBase64Url(nonce),
    ciphertext: encodeBase64Url(ciphertext),
  };
};

export const decryptPayload = (key: Uint8Array, deviceId: string, seq: number, envelope: E2eeEnvelope) => {
  const nonce = decodeBase64(envelope.nonce);
  const ciphertext = decodeBase64(envelope.ciphertext);
  const aad = buildAad(deviceId, seq);
  const cipher = new XChaCha20Poly1305(key);
  const out = cipher.open(nonce, ciphertext, aad);
  if (!out) throw new Error("decrypt failed");
  return out;
};

export const encodeBase64Url = (bytes: Uint8Array): string => {
  return Buffer.from(bytes)
    .toString("base64")
    .replace(/\+/g, "-")
    .replace(/\//g, "_")
    .replace(/=+$/g, "");
};

export const decodeBase64 = (value: string): Uint8Array => {
  let normalized = value.trim().replace(/-/g, "+").replace(/_/g, "/");
  while (normalized.length % 4 !== 0) {
    normalized += "=";
  }
  return new Uint8Array(Buffer.from(normalized, "base64"));
};

const randomBytes = (len: number): Uint8Array => {
  const out = new Uint8Array(len);
  globalThis.crypto.getRandomValues(out);
  return out;
};

const sha256Bytes = (input: string): Uint8Array => {
  const hash = new SHA256();
  hash.update(new TextEncoder().encode(input));
  return hash.digest();
};

const buildAad = (deviceId: string, seq: number): Uint8Array =>
  new TextEncoder().encode(`${deviceId}:${seq}`);
