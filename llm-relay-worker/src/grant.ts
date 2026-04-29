import { RelayHttpError, requireString } from "./http";
import type { Env, RelayDelegation, RunGrant } from "./types";

const RELAY_AUDIENCE = "ctx-llm-relay";
const CONTROL_PLANE_ISSUER = "ctx-control-plane";

export interface VerifiedGrantEnvelope {
  delegation: RelayDelegation;
  grant: RunGrant;
  delegationJws?: string;
  grantJws?: string;
}

export async function verifyGrantEnvelope(
  request: Request,
  env: Env,
): Promise<VerifiedGrantEnvelope> {
  const mode = env.GRANT_VERIFICATION_MODE ?? "signed_chain";
  let delegation: RelayDelegation;
  let grant: RunGrant;
  let delegationJws: string | undefined;
  let grantJws: string | undefined;
  if (mode === "test_unsigned") {
    if (!allowsUnsafeGrantVerificationMode(env)) {
      throw new RelayHttpError(500, "invalid_config", "test unsigned grants are forbidden outside dev/test");
    }
    delegation = decodeHeaderJson<RelayDelegation>(request, "x-ctx-relay-delegation");
    grant = decodeHeaderJson<RunGrant>(request, "x-ctx-relay-grant");
  } else if (mode === "hmac") {
    if (!allowsUnsafeGrantVerificationMode(env)) {
      throw new RelayHttpError(500, "invalid_config", "hmac grant verification is forbidden outside dev/test");
    }
    delegation = decodeHeaderJson<RelayDelegation>(request, "x-ctx-relay-delegation");
    grant = decodeHeaderJson<RunGrant>(request, "x-ctx-relay-grant");
    const secret = requireString(env.GRANT_HMAC_SECRET, "GRANT_HMAC_SECRET");
    const signature = requireString(request.headers.get("x-ctx-relay-signature"), "x-ctx-relay-signature");
    const signed = [
      request.headers.get("x-ctx-relay-delegation") ?? "",
      request.headers.get("x-ctx-relay-grant") ?? "",
    ].join(".");
    const expected = await hmacSha256Base64Url(secret, signed);
    if (!timingSafeEqual(signature, expected)) {
      throw new RelayHttpError(401, "invalid_grant_signature", "grant signature is invalid");
    }
  } else if (mode === "signed_chain") {
    const signed = await verifySignedChain(request, env);
    delegation = signed.delegation;
    grant = signed.grant;
    delegationJws = signed.delegationJws;
    grantJws = signed.grantJws;
  } else {
    throw new RelayHttpError(500, "invalid_config", "unsupported grant verification mode");
  }
  validateGrantSubset(delegation, grant);
  return { delegation, grant, delegationJws, grantJws };
}

async function verifySignedChain(
  request: Request,
  env: Env,
): Promise<Required<Pick<VerifiedGrantEnvelope, "delegation" | "grant" | "delegationJws" | "grantJws">>> {
  const delegationJws = requireString(
    request.headers.get("x-ctx-relay-delegation-jws"),
    "x-ctx-relay-delegation-jws",
  );
  const delegation = await verifyCompactJws<RelayDelegation>(
    delegationJws,
    parseJwks(requireString(env.CONTROL_PLANE_JWKS, "CONTROL_PLANE_JWKS")),
    "relay delegation",
  );
  const grantJws = requireString(
    request.headers.get("x-ctx-relay-grant-jws"),
    "x-ctx-relay-grant-jws",
  );
  const daemonPublicKey = requirePublicJwk(delegation.daemon_public_key_jwk, "daemon_public_key_jwk");
  const actualThumbprint = await jwkThumbprint(daemonPublicKey);
  if (actualThumbprint !== delegation.daemon_public_key_thumbprint) {
    throw new RelayHttpError(401, "invalid_grant_signature", "daemon public key thumbprint mismatch");
  }
  const grant = await verifyCompactJws<RunGrant>(grantJws, [daemonPublicKey], "run grant");
  return { delegation, grant, delegationJws, grantJws };
}

interface CompactJwsHeader {
  alg: "ES256" | "EdDSA";
  kid?: string;
  typ?: string;
}

async function verifyCompactJws<T>(
  compact: string,
  keys: JsonWebKey[],
  label: string,
): Promise<T> {
  const parts = compact.split(".");
  if (parts.length !== 3 || parts.some((part) => part.length === 0)) {
    throw new RelayHttpError(401, "invalid_grant", `${label} must be compact JWS`);
  }
  const header = parseJwsHeader(decodeJson(parts[0], `${label} header`));
  const key = selectJwk(keys, header, label);
  const cryptoKey = await importVerifyKey(key, header.alg);
  const signingInput = `${parts[0]}.${parts[1]}`;
  const ok = await crypto.subtle.verify(
    verifyAlgorithm(header.alg),
    cryptoKey,
    toArrayBuffer(base64UrlDecode(parts[2])),
    new TextEncoder().encode(signingInput),
  );
  if (!ok) {
    throw new RelayHttpError(401, "invalid_grant_signature", `${label} signature is invalid`);
  }
  return decodeJson<T>(parts[1], `${label} payload`);
}

function parseJwsHeader(value: unknown): CompactJwsHeader {
  if (!isRecord(value)) {
    throw new RelayHttpError(401, "invalid_grant", "JWS header must be an object");
  }
  if (value.alg !== "ES256" && value.alg !== "EdDSA") {
    throw new RelayHttpError(401, "invalid_grant", "JWS alg must be ES256 or EdDSA");
  }
  const header: CompactJwsHeader = { alg: value.alg };
  if (typeof value.kid === "string" && value.kid.length > 0) {
    header.kid = value.kid;
  }
  if (typeof value.typ === "string" && value.typ.length > 0) {
    header.typ = value.typ;
  }
  return header;
}

function parseJwks(value: string): JsonWebKey[] {
  let parsed: unknown;
  try {
    parsed = JSON.parse(value) as unknown;
  } catch {
    throw new RelayHttpError(500, "invalid_config", "CONTROL_PLANE_JWKS must be valid JSON");
  }
  if (Array.isArray(parsed)) {
    return parsed.map((entry) => requirePublicJwk(entry, "CONTROL_PLANE_JWKS", "invalid_config", 500));
  }
  if (isRecord(parsed) && Array.isArray(parsed.keys)) {
    return parsed.keys.map((entry) => requirePublicJwk(entry, "CONTROL_PLANE_JWKS.keys", "invalid_config", 500));
  }
  return [requirePublicJwk(parsed, "CONTROL_PLANE_JWKS", "invalid_config", 500)];
}

function requirePublicJwk(
  value: unknown,
  field: string,
  code = "invalid_grant",
  status = 401,
): JsonWebKey {
  if (!isRecord(value) || typeof value.kty !== "string") {
    throw new RelayHttpError(status, code, `${field} must be a public JWK`);
  }
  if ("d" in value) {
    throw new RelayHttpError(status, code, `${field} must not include private key material`);
  }
  return value as JsonWebKey;
}

function selectJwk(keys: JsonWebKey[], header: CompactJwsHeader, label: string): JsonWebKey {
  const candidates = header.kid == null ? keys : keys.filter((key) => jwkKid(key) === header.kid);
  if (candidates.length !== 1) {
    throw new RelayHttpError(401, "invalid_grant", `${label} signing key is ambiguous or missing`);
  }
  const [key] = candidates;
  if (key.alg != null && key.alg !== header.alg) {
    throw new RelayHttpError(401, "invalid_grant", `${label} signing key alg mismatch`);
  }
  return key;
}

function jwkKid(jwk: JsonWebKey): string | undefined {
  const value = jwk as Record<string, unknown>;
  return typeof value.kid === "string" ? value.kid : undefined;
}

async function importVerifyKey(jwk: JsonWebKey, alg: CompactJwsHeader["alg"]): Promise<CryptoKey> {
  if (alg === "ES256") {
    return crypto.subtle.importKey(
      "jwk",
      jwk,
      { name: "ECDSA", namedCurve: "P-256" },
      false,
      ["verify"],
    );
  }
  return crypto.subtle.importKey("jwk", jwk, { name: "Ed25519" }, false, ["verify"]);
}

function verifyAlgorithm(alg: CompactJwsHeader["alg"]): AlgorithmIdentifier | EcdsaParams {
  if (alg === "ES256") {
    return { name: "ECDSA", hash: "SHA-256" };
  }
  return { name: "Ed25519" };
}

function decodeJson<T = unknown>(value: string, field: string): T {
  try {
    return JSON.parse(new TextDecoder().decode(base64UrlDecode(value))) as T;
  } catch {
    throw new RelayHttpError(401, "invalid_grant", `${field} is not valid base64url JSON`);
  }
}

export async function jwkThumbprint(jwk: JsonWebKey): Promise<string> {
  const canonical = canonicalJwkThumbprintInput(jwk);
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(canonical));
  return base64UrlEncode(new Uint8Array(digest));
}

function canonicalJwkThumbprintInput(jwk: JsonWebKey): string {
  if (jwk.kty === "EC" && jwk.crv === "P-256" && typeof jwk.x === "string" && typeof jwk.y === "string") {
    return JSON.stringify({ crv: jwk.crv, kty: jwk.kty, x: jwk.x, y: jwk.y });
  }
  if (jwk.kty === "OKP" && jwk.crv === "Ed25519" && typeof jwk.x === "string") {
    return JSON.stringify({ crv: jwk.crv, kty: jwk.kty, x: jwk.x });
  }
  throw new RelayHttpError(401, "invalid_grant", "unsupported daemon public key JWK");
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function allowsUnsafeGrantVerificationMode(env: Env): boolean {
  const environment = env.ENVIRONMENT;
  return environment === "dev" || environment === "test" || environment === "local";
}

function decodeHeaderJson<T>(request: Request, header: string): T {
  const value = request.headers.get(header);
  if (value == null || value.trim() === "") {
    throw new RelayHttpError(401, "missing_grant", `${header} is required`);
  }
  try {
    return JSON.parse(new TextDecoder().decode(base64UrlDecode(value))) as T;
  } catch {
    throw new RelayHttpError(401, "invalid_grant", `${header} is not valid base64url JSON`);
  }
}

function validateGrantSubset(delegation: RelayDelegation, grant: RunGrant): void {
  const now = Date.now();
  requireEqual(delegation.issuer, CONTROL_PLANE_ISSUER, "delegation issuer");
  requireEqual(delegation.audience, RELAY_AUDIENCE, "delegation audience");
  requireEqual(grant.audience, RELAY_AUDIENCE, "grant audience");
  requireEqual(grant.route_type, "ctx_managed", "grant route_type");
  requireEqual(grant.delegation_jti, delegation.jti, "grant delegation_jti");
  requireEqual(grant.access_context_kind, delegation.access_context_kind, "access context");
  requireEqual(grant.billing_subject_id, delegation.billing_subject_id, "billing subject");
  requireEqual(grant.ctx_user_id, delegation.ctx_user_id, "ctx user");
  requireEqual(grant.ctx_account_id ?? "", delegation.ctx_account_id ?? "", "ctx account");
  requireEqual(grant.ctx_org_id ?? "", delegation.ctx_org_id ?? "", "ctx org");
  requireEqual(grant.ctx_membership_id ?? "", delegation.ctx_membership_id ?? "", "ctx membership");
  requireEqual(grant.daemon_id, delegation.daemon_id, "daemon");
  requireEqual(grant.policy_version, delegation.policy_version, "policy version");
  requireEqual(grant.pricing_version, delegation.pricing_version, "pricing version");
  if (Date.parse(delegation.expires_at) <= now || Date.parse(grant.expires_at) <= now) {
    throw new RelayHttpError(403, "expired_grant", "grant or delegation has expired");
  }
  if (Date.parse(grant.expires_at) > Date.parse(delegation.expires_at)) {
    throw new RelayHttpError(403, "invalid_grant", "grant expires after delegation");
  }
  if (!delegation.allowed_route_ids.includes(grant.route_id)) {
    throw new RelayHttpError(403, "route_not_allowed", "route is not allowed");
  }
  if (!delegation.allowed_auth_methods.every((method) => method === "ctx_provider_key")) {
    throw new RelayHttpError(403, "invalid_delegation", "delegation must only allow ctx_provider_key");
  }
  if (!delegation.allowed_provider_model_pairs.some((pair) => pair.provider_id === grant.provider_id && pair.model_id === grant.model_id)) {
    throw new RelayHttpError(403, "model_not_allowed", "provider/model pair is not allowed");
  }
  if (grant.max_estimated_cents > delegation.max_per_request_cents) {
    throw new RelayHttpError(403, "grant_limit_exceeded", "grant exceeds per-request limit");
  }
  if ((grant.max_input_tokens ?? 0) > delegation.max_input_tokens) {
    throw new RelayHttpError(403, "grant_limit_exceeded", "grant exceeds input token limit");
  }
  if ((grant.max_output_tokens ?? 0) > delegation.max_output_tokens) {
    throw new RelayHttpError(403, "grant_limit_exceeded", "grant exceeds output token limit");
  }
}

function requireEqual(left: string, right: string, field: string): void {
  if (left !== right) {
    throw new RelayHttpError(403, "invalid_grant", `${field} mismatch`);
  }
}

async function hmacSha256Base64Url(secret: string, value: string): Promise<string> {
  const key = await crypto.subtle.importKey(
    "raw",
    new TextEncoder().encode(secret),
    { name: "HMAC", hash: "SHA-256" },
    false,
    ["sign"],
  );
  const signature = await crypto.subtle.sign("HMAC", key, new TextEncoder().encode(value));
  return base64UrlEncode(new Uint8Array(signature));
}

function timingSafeEqual(left: string, right: string): boolean {
  const leftBytes = new TextEncoder().encode(left);
  const rightBytes = new TextEncoder().encode(right);
  if (leftBytes.length !== rightBytes.length) {
    return false;
  }
  let diff = 0;
  for (let index = 0; index < leftBytes.length; index += 1) {
    diff |= leftBytes[index] ^ rightBytes[index];
  }
  return diff === 0;
}

export function base64UrlEncode(bytes: Uint8Array): string {
  let binary = "";
  for (const byte of bytes) {
    binary += String.fromCharCode(byte);
  }
  return btoa(binary).replaceAll("+", "-").replaceAll("/", "_").replaceAll("=", "");
}

function base64UrlDecode(value: string): Uint8Array {
  const padded = value.replaceAll("-", "+").replaceAll("_", "/").padEnd(Math.ceil(value.length / 4) * 4, "=");
  const binary = atob(padded);
  const bytes = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) {
    bytes[index] = binary.charCodeAt(index);
  }
  return bytes;
}

function toArrayBuffer(bytes: Uint8Array): ArrayBuffer {
  const copy = new Uint8Array(bytes.byteLength);
  copy.set(bytes);
  return copy.buffer as ArrayBuffer;
}
