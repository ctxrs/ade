#!/usr/bin/env node

const crypto = require("node:crypto");
const fs = require("node:fs");

const DEFAULT_PUBLIC_STORAGE_ORIGIN = "https://api.ctx.rs";
const DEFAULT_PUBLIC_STORAGE_BUCKET = "releases";
const DEFAULT_R2_REGION = "auto";

function trimValue(value) {
  return String(value || "").trim();
}

function requireValue(name, value) {
  const normalized = trimValue(value);
  if (!normalized) {
    throw new Error(`missing required release storage value: ${name}`);
  }
  return normalized;
}

function normalizeProvider(value) {
  const provider = trimValue(value || "r2").toLowerCase();
  if (provider !== "supabase" && provider !== "r2") {
    throw new Error(`unsupported RELEASE_STORAGE_PROVIDER '${value}' (expected supabase or r2)`);
  }
  return provider;
}

function resolveStorageProvider(env = process.env) {
  return normalizeProvider(
    env.RELEASE_STORAGE_PROVIDER
      || env.CTX_RELEASE_STORAGE_PROVIDER
      || env.CTX_RELEASE_STORAGE_BACKEND
      || "r2",
  );
}

function resolveStorageBucket(env = process.env, provider = resolveStorageProvider(env)) {
  if (provider === "r2") {
    return trimValue(
      env.RELEASE_STORAGE_BUCKET
        || env.CTX_RELEASES_R2_BUCKET
        || env.CTX_RELEASE_R2_BUCKET
        || env.RELEASE_R2_BUCKET,
    );
  }
  return trimValue(
    env.RELEASE_STORAGE_BUCKET
      || env.SUPABASE_STORAGE_BUCKET,
  );
}

function resolvePublicStorageOrigin(env = process.env) {
  return trimValue(env.RELEASE_PUBLIC_STORAGE_ORIGIN || env.SUPABASE_PUBLIC_URL || DEFAULT_PUBLIC_STORAGE_ORIGIN)
    .replace(/\/+$/, "");
}

function resolvePublicStorageBucket(env = process.env, provider = resolveStorageProvider(env), bucket = "") {
  if (env.RELEASE_PUBLIC_STORAGE_BUCKET != null && trimValue(env.RELEASE_PUBLIC_STORAGE_BUCKET)) {
    return trimValue(env.RELEASE_PUBLIC_STORAGE_BUCKET);
  }
  if (provider === "supabase" && trimValue(bucket)) {
    return trimValue(bucket);
  }
  return DEFAULT_PUBLIC_STORAGE_BUCKET;
}

function resolveR2Endpoint(env = process.env) {
  const explicit = trimValue(env.RELEASE_R2_ENDPOINT || env.CTX_RELEASE_R2_ENDPOINT);
  if (explicit) {
    return explicit.replace(/\/+$/, "");
  }
  const accountId = trimValue(env.RELEASE_R2_ACCOUNT_ID || env.CTX_RELEASE_R2_ACCOUNT_ID);
  if (!accountId) {
    return "";
  }
  return `https://${accountId}.r2.cloudflarestorage.com`;
}

function resolveStorageConfigFromEnv(env = process.env) {
  const provider = resolveStorageProvider(env);
  const bucket = requireValue("RELEASE_STORAGE_BUCKET", resolveStorageBucket(env, provider));
  const publicOrigin = resolvePublicStorageOrigin(env);
  const publicBucket = requireValue("RELEASE_PUBLIC_STORAGE_BUCKET", resolvePublicStorageBucket(env, provider, bucket));
  if (provider === "supabase") {
    return {
      provider,
      bucket,
      publicBucket,
      publicOrigin,
      supabase: {
        serviceRoleKey: requireValue("SUPABASE_SERVICE_ROLE_KEY", env.SUPABASE_SERVICE_ROLE_KEY),
        url: requireValue("SUPABASE_URL", env.SUPABASE_URL).replace(/\/+$/, ""),
      },
    };
  }
  return {
    provider,
    bucket,
    publicBucket,
    publicOrigin,
    r2: {
      accessKeyId: requireValue(
        "RELEASE_R2_ACCESS_KEY_ID",
        env.RELEASE_R2_ACCESS_KEY_ID || env.CTX_RELEASE_R2_ACCESS_KEY_ID,
      ),
      endpoint: requireValue("RELEASE_R2_ENDPOINT", resolveR2Endpoint(env)),
      region: trimValue(env.RELEASE_R2_REGION || env.CTX_RELEASE_R2_REGION || DEFAULT_R2_REGION),
      secretAccessKey: requireValue(
        "RELEASE_R2_SECRET_ACCESS_KEY",
        env.RELEASE_R2_SECRET_ACCESS_KEY || env.CTX_RELEASE_R2_SECRET_ACCESS_KEY,
      ),
      sessionToken: trimValue(env.RELEASE_R2_SESSION_TOKEN || env.CTX_RELEASE_R2_SESSION_TOKEN),
    },
  };
}

function encodeRfc3986(value) {
  return encodeURIComponent(value).replace(/[!'()*]/g, (char) =>
    `%${char.charCodeAt(0).toString(16).toUpperCase()}`);
}

function normalizeObjectPath(objectPath) {
  const normalized = trimValue(objectPath).replace(/^\/+/, "");
  if (!normalized) {
    throw new Error("objectPath is required");
  }
  if (normalized.includes("..")) {
    throw new Error(`objectPath must not contain '..': ${objectPath}`);
  }
  return normalized;
}

function buildPublicObjectUrl(config, objectPath) {
  const publicBucket = config.publicBucket || config.bucket;
  return `${config.publicOrigin}/storage/v1/object/public/${publicBucket}/${normalizeObjectPath(objectPath)}`;
}

function buildSupabaseObjectUrl(config, objectPath) {
  return `${config.supabase.url}/storage/v1/object/${config.bucket}/${normalizeObjectPath(objectPath)}`;
}

function buildR2ObjectRequestUrl(config, objectPath) {
  const endpoint = new URL(config.r2.endpoint);
  const origin = `${endpoint.protocol}//${endpoint.host}`;
  const basePath = endpoint.pathname.replace(/\/+$/, "");
  const encodedSegments = [
    encodeRfc3986(config.bucket),
    ...normalizeObjectPath(objectPath).split("/").map((segment) => encodeRfc3986(segment)),
  ];
  const canonicalUri = `${basePath}/${encodedSegments.join("/")}`.replace(/\/{2,}/g, "/");
  return {
    canonicalUri,
    url: `${origin}${canonicalUri}`,
  };
}

function sha256Hex(bytes) {
  return crypto.createHash("sha256").update(bytes).digest("hex");
}

function hmac(key, value, encoding) {
  return crypto.createHmac("sha256", key).update(value).digest(encoding);
}

function toAmzDate(now = new Date()) {
  return now.toISOString().replace(/[:-]|\.\d{3}/g, "");
}

function signR2Request(config, {
  body = Buffer.alloc(0),
  headers = {},
  method,
  objectPath,
  now = new Date(),
}) {
  const payload = Buffer.isBuffer(body) ? body : Buffer.from(String(body));
  const request = buildR2ObjectRequestUrl(config, objectPath);
  const url = new URL(request.url);
  const amzDate = toAmzDate(now);
  const dateStamp = amzDate.slice(0, 8);
  const payloadHash = sha256Hex(payload);
  const normalizedHeaders = {
    ...Object.fromEntries(
      Object.entries(headers)
        .filter(([, value]) => value !== undefined && value !== null && String(value).trim() !== "")
        .map(([key, value]) => [key.toLowerCase(), String(value).trim()]),
    ),
    host: url.host,
    "x-amz-content-sha256": payloadHash,
    "x-amz-date": amzDate,
  };
  if (config.r2.sessionToken) {
    normalizedHeaders["x-amz-security-token"] = config.r2.sessionToken;
  }
  const signedHeaderNames = Object.keys(normalizedHeaders).sort();
  const canonicalHeaders = signedHeaderNames
    .map((name) => `${name}:${normalizedHeaders[name].replace(/\s+/g, " ")}\n`)
    .join("");
  const signedHeaders = signedHeaderNames.join(";");
  const canonicalRequest = [
    method.toUpperCase(),
    request.canonicalUri,
    "",
    canonicalHeaders,
    signedHeaders,
    payloadHash,
  ].join("\n");
  const credentialScope = `${dateStamp}/${config.r2.region}/s3/aws4_request`;
  const stringToSign = [
    "AWS4-HMAC-SHA256",
    amzDate,
    credentialScope,
    sha256Hex(Buffer.from(canonicalRequest)),
  ].join("\n");
  const dateKey = hmac(`AWS4${config.r2.secretAccessKey}`, dateStamp);
  const regionKey = hmac(dateKey, config.r2.region);
  const serviceKey = hmac(regionKey, "s3");
  const signingKey = hmac(serviceKey, "aws4_request");
  const signature = hmac(signingKey, stringToSign, "hex");
  normalizedHeaders.authorization = [
    "AWS4-HMAC-SHA256",
    `Credential=${config.r2.accessKeyId}/${credentialScope},`,
    `SignedHeaders=${signedHeaders},`,
    `Signature=${signature}`,
  ].join(" ");
  return {
    body: payload,
    headers: normalizedHeaders,
    url: request.url,
  };
}

function buildStorageClientFromEnv(env = process.env, { fetchImpl = globalThis.fetch } = {}) {
  return new ReleaseStorageClient(resolveStorageConfigFromEnv(env), { fetchImpl });
}

function isSupabaseDuplicate(status, text) {
  return status === 409 || (status === 400 && /duplicate|already exists|exists/i.test(text));
}

function isMissingObject(status, text) {
  return status === 404 || (status === 400 && /not[_ -]?found|does not exist|no such|NoSuchKey/i.test(text));
}

async function responseText(response) {
  try {
    return await response.text();
  } catch {
    return "";
  }
}

class ReleaseStorageClient {
  constructor(config, { fetchImpl = globalThis.fetch } = {}) {
    if (typeof fetchImpl !== "function") {
      throw new Error("fetch is required for release storage");
    }
    this.config = config;
    this.fetchImpl = fetchImpl;
  }

  publicObjectUrl(objectPath) {
    return buildPublicObjectUrl(this.config, objectPath);
  }

  async getObjectBuffer(objectPath, { allowMissing = false } = {}) {
    if (this.config.provider === "r2") {
      return this.getR2ObjectBuffer(objectPath, { allowMissing });
    }
    return this.getSupabaseObjectBuffer(objectPath, { allowMissing });
  }

  async getObjectText(objectPath, options = {}) {
    const bytes = await this.getObjectBuffer(objectPath, options);
    return bytes === null ? null : bytes.toString("utf8");
  }

  async getSupabaseObjectBuffer(objectPath, { allowMissing = false } = {}) {
    const response = await this.fetchImpl(`${buildSupabaseObjectUrl(this.config, objectPath)}?cb=${Date.now()}`, {
      headers: {
        apikey: this.config.supabase.serviceRoleKey,
        authorization: `Bearer ${this.config.supabase.serviceRoleKey}`,
      },
      method: "GET",
    });
    const arrayBuffer = response.ok ? await response.arrayBuffer() : null;
    if (response.ok) {
      return Buffer.from(arrayBuffer);
    }
    const text = await responseText(response);
    if (allowMissing && isMissingObject(response.status, text)) {
      return null;
    }
    throw new Error(`failed to read ${normalizeObjectPath(objectPath)} (HTTP ${response.status}): ${text.slice(0, 500)}`);
  }

  async getR2ObjectBuffer(objectPath, { allowMissing = false } = {}) {
    const signed = signR2Request(this.config, {
      method: "GET",
      objectPath,
    });
    const response = await this.fetchImpl(signed.url, {
      headers: signed.headers,
      method: "GET",
    });
    if (response.ok) {
      return Buffer.from(await response.arrayBuffer());
    }
    const text = await responseText(response);
    if (allowMissing && isMissingObject(response.status, text)) {
      return null;
    }
    throw new Error(`failed to read ${normalizeObjectPath(objectPath)} from R2 (HTTP ${response.status}): ${text.slice(0, 500)}`);
  }

  async putObject({
    body,
    contentType = "application/octet-stream",
    objectPath,
    upsert = false,
    verifyExisting = false,
  }) {
    const bytes = Buffer.isBuffer(body) ? body : Buffer.from(String(body));
    if (this.config.provider === "r2") {
      return this.putR2Object({ body: bytes, contentType, objectPath, upsert, verifyExisting });
    }
    return this.putSupabaseObject({ body: bytes, contentType, objectPath, upsert, verifyExisting });
  }

  async putSupabaseObject({ body, contentType, objectPath, upsert, verifyExisting }) {
    const response = await this.fetchImpl(buildSupabaseObjectUrl(this.config, objectPath), {
      body,
      headers: {
        apikey: this.config.supabase.serviceRoleKey,
        authorization: `Bearer ${this.config.supabase.serviceRoleKey}`,
        "content-type": contentType,
        "x-upsert": upsert ? "true" : "false",
      },
      method: "POST",
    });
    if (response.ok) {
      return { existing: false, objectPath: normalizeObjectPath(objectPath), uploaded: true };
    }
    const text = await responseText(response);
    if (!upsert && verifyExisting && isSupabaseDuplicate(response.status, text)) {
      await this.verifyExistingObject(objectPath, body);
      return { existing: true, objectPath: normalizeObjectPath(objectPath), uploaded: false };
    }
    throw new Error(`failed to upload ${normalizeObjectPath(objectPath)} (HTTP ${response.status}): ${text.slice(0, 500)}`);
  }

  async putR2Object({ body, contentType, objectPath, upsert, verifyExisting }) {
    const headers = {
      "content-type": contentType,
      ...(upsert ? {} : { "if-none-match": "*" }),
    };
    const signed = signR2Request(this.config, {
      body,
      headers,
      method: "PUT",
      objectPath,
    });
    const response = await this.fetchImpl(signed.url, {
      body: signed.body,
      headers: signed.headers,
      method: "PUT",
    });
    if (response.ok) {
      return { existing: false, objectPath: normalizeObjectPath(objectPath), uploaded: true };
    }
    const text = await responseText(response);
    if (!upsert && verifyExisting && (response.status === 409 || response.status === 412)) {
      await this.verifyExistingObject(objectPath, body);
      return { existing: true, objectPath: normalizeObjectPath(objectPath), uploaded: false };
    }
    throw new Error(`failed to upload ${normalizeObjectPath(objectPath)} to R2 (HTTP ${response.status}): ${text.slice(0, 500)}`);
  }

  async verifyExistingObject(objectPath, expectedBytes) {
    const existing = await this.getObjectBuffer(objectPath);
    const expectedSha = sha256Hex(expectedBytes);
    const existingSha = sha256Hex(existing);
    if (expectedSha !== existingSha) {
      throw new Error(
        `immutable upload conflict for ${normalizeObjectPath(objectPath)} has mismatched existing content (local sha256 ${expectedSha}, existing sha256 ${existingSha})`,
      );
    }
  }

  async deleteObject(objectPath) {
    if (this.config.provider === "r2") {
      return this.deleteR2Object(objectPath);
    }
    return this.deleteSupabaseObject(objectPath);
  }

  async deleteSupabaseObject(objectPath) {
    const response = await this.fetchImpl(buildSupabaseObjectUrl(this.config, objectPath), {
      headers: {
        apikey: this.config.supabase.serviceRoleKey,
        authorization: `Bearer ${this.config.supabase.serviceRoleKey}`,
      },
      method: "DELETE",
    });
    if (response.ok || response.status === 404) {
      return { deleted: response.ok, objectPath: normalizeObjectPath(objectPath) };
    }
    const text = await responseText(response);
    throw new Error(`failed to delete ${normalizeObjectPath(objectPath)} (HTTP ${response.status}): ${text.slice(0, 500)}`);
  }

  async deleteR2Object(objectPath) {
    const signed = signR2Request(this.config, {
      method: "DELETE",
      objectPath,
    });
    const response = await this.fetchImpl(signed.url, {
      headers: signed.headers,
      method: "DELETE",
    });
    if (response.ok || response.status === 404) {
      return { deleted: response.ok, objectPath: normalizeObjectPath(objectPath) };
    }
    const text = await responseText(response);
    throw new Error(`failed to delete ${normalizeObjectPath(objectPath)} from R2 (HTTP ${response.status}): ${text.slice(0, 500)}`);
  }

  async ensureBucket({ fileSizeLimitBytes = "5368709120" } = {}) {
    if (this.config.provider === "r2") {
      return { bucket: this.config.bucket, provider: "r2", skipped: true };
    }
    const payload = {
      file_size_limit: Number(fileSizeLimitBytes),
      id: this.config.bucket,
      name: this.config.bucket,
      public: true,
    };
    const headers = {
      apikey: this.config.supabase.serviceRoleKey,
      authorization: `Bearer ${this.config.supabase.serviceRoleKey}`,
      "content-type": "application/json",
    };
    const read = await this.fetchImpl(`${this.config.supabase.url}/storage/v1/bucket/${this.config.bucket}`, {
      headers,
      method: "GET",
    });
    if (read.ok) {
      const bucket = await read.json().catch(() => ({}));
      const rawLimit = bucket && Object.prototype.hasOwnProperty.call(bucket, "file_size_limit")
        ? bucket.file_size_limit
        : null;
      const limit = rawLimit == null || rawLimit === "" ? null : Number(rawLimit);
      if (bucket?.public === true && (limit == null || limit >= payload.file_size_limit)) {
        return { bucket: this.config.bucket, provider: "supabase", skipped: false };
      }
    }
    const create = await this.fetchImpl(`${this.config.supabase.url}/storage/v1/bucket`, {
      body: JSON.stringify(payload),
      headers,
      method: "POST",
    });
    if (create.ok) {
      return { bucket: this.config.bucket, provider: "supabase", skipped: false };
    }
    for (const method of ["PUT", "PATCH"]) {
      const update = await this.fetchImpl(`${this.config.supabase.url}/storage/v1/bucket/${this.config.bucket}`, {
        body: JSON.stringify(payload),
        headers,
        method,
      });
      if (update.ok || update.status === 204) {
        return { bucket: this.config.bucket, provider: "supabase", skipped: false };
      }
    }
    throw new Error(`failed to ensure Supabase bucket '${this.config.bucket}' is public`);
  }
}

function readFileBytes(filePath) {
  return fs.readFileSync(filePath);
}

module.exports = {
  DEFAULT_PUBLIC_STORAGE_ORIGIN,
  DEFAULT_PUBLIC_STORAGE_BUCKET,
  DEFAULT_R2_REGION,
  ReleaseStorageClient,
  buildPublicObjectUrl,
  buildR2ObjectRequestUrl,
  buildStorageClientFromEnv,
  normalizeObjectPath,
  normalizeProvider,
  resolveStorageProvider,
  readFileBytes,
  resolvePublicStorageOrigin,
  resolvePublicStorageBucket,
  resolveStorageBucket,
  resolveStorageConfigFromEnv,
  sha256Hex,
  signR2Request,
};
