#!/usr/bin/env node

const crypto = require("node:crypto");
const fs = require("node:fs");
const path = require("node:path");
const { Readable } = require("node:stream");
const { pipeline } = require("node:stream/promises");

const DEFAULT_PUBLIC_STORAGE_ORIGIN = "https://api.ctx.rs";
const DEFAULT_PUBLIC_STORAGE_BUCKET = "releases";
const DEFAULT_R2_REGION = "auto";
const DEFAULT_R2_PUT_ATTEMPTS = 3;
const DEFAULT_R2_PUT_RETRY_BASE_DELAY_MS = 1000;
const DEFAULT_R2_GET_TIMEOUT_MS = 15 * 60 * 1000;
const DEFAULT_R2_PUT_TIMEOUT_MS = 15 * 60 * 1000;

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
  if (provider !== "r2") {
    throw new Error(`unsupported RELEASE_STORAGE_PROVIDER '${value}' (expected r2)`);
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
  if (provider !== "r2") {
    throw new Error(`unsupported release storage provider '${provider}'`);
  }
  return trimValue(
    env.RELEASE_STORAGE_BUCKET
      || env.CTX_RELEASES_R2_BUCKET
      || env.CTX_RELEASE_R2_BUCKET
      || env.RELEASE_R2_BUCKET,
  );
}

function resolvePublicStorageOrigin(env = process.env) {
  return trimValue(env.RELEASE_PUBLIC_STORAGE_ORIGIN || DEFAULT_PUBLIC_STORAGE_ORIGIN)
    .replace(/\/+$/, "");
}

function resolvePublicStorageBucket(env = process.env) {
  if (env.RELEASE_PUBLIC_STORAGE_BUCKET != null && trimValue(env.RELEASE_PUBLIC_STORAGE_BUCKET)) {
    return trimValue(env.RELEASE_PUBLIC_STORAGE_BUCKET);
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

function sha256FileHex(filePath) {
  return new Promise((resolve, reject) => {
    const hash = crypto.createHash("sha256");
    const stream = fs.createReadStream(filePath);
    stream.on("data", (chunk) => hash.update(chunk));
    stream.on("error", reject);
    stream.on("end", () => resolve(hash.digest("hex")));
  });
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
  payloadHash: explicitPayloadHash,
}) {
  const payload = Buffer.isBuffer(body) ? body : Buffer.from(String(body));
  const request = buildR2ObjectRequestUrl(config, objectPath);
  const url = new URL(request.url);
  const amzDate = toAmzDate(now);
  const dateStamp = amzDate.slice(0, 8);
  const payloadHash = explicitPayloadHash || sha256Hex(payload);
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

function isMissingObject(status, text) {
  return status === 404 || (status === 400 && /not[_ -]?found|does not exist|no such|NoSuchKey/i.test(text));
}

function parsePositiveInteger(value, fallback, name) {
  if (value === undefined || value === null || value === "") {
    return fallback;
  }
  const parsed = Number(value);
  if (!Number.isInteger(parsed) || parsed < 1) {
    throw new Error(`${name} must be a positive integer`);
  }
  return parsed;
}

function parseNonNegativeInteger(value, fallback, name) {
  if (value === undefined || value === null || value === "") {
    return fallback;
  }
  const parsed = Number(value);
  if (!Number.isInteger(parsed) || parsed < 0) {
    throw new Error(`${name} must be a non-negative integer`);
  }
  return parsed;
}

function isRetryableR2Status(status) {
  return status === 408 || status === 425 || status === 429 || status >= 500;
}

function sleep(ms) {
  if (ms <= 0) {
    return Promise.resolve();
  }
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function responseText(response) {
  try {
    return await response.text();
  } catch {
    return "";
  }
}

class ReleaseStorageClient {
  constructor(config, {
    fetchImpl = globalThis.fetch,
    putRetryAttempts = process.env.RELEASE_STORAGE_PUT_ATTEMPTS,
    putRetryBaseDelayMs = process.env.RELEASE_STORAGE_PUT_RETRY_BASE_DELAY_MS,
    getTimeoutMs = process.env.RELEASE_STORAGE_GET_TIMEOUT_MS,
    putTimeoutMs = process.env.RELEASE_STORAGE_PUT_TIMEOUT_MS,
  } = {}) {
    if (typeof fetchImpl !== "function") {
      throw new Error("fetch is required for release storage");
    }
    this.config = config;
    this.fetchImpl = fetchImpl;
    this.putRetryAttempts = parsePositiveInteger(
      putRetryAttempts,
      DEFAULT_R2_PUT_ATTEMPTS,
      "RELEASE_STORAGE_PUT_ATTEMPTS",
    );
    this.putRetryBaseDelayMs = parseNonNegativeInteger(
      putRetryBaseDelayMs,
      DEFAULT_R2_PUT_RETRY_BASE_DELAY_MS,
      "RELEASE_STORAGE_PUT_RETRY_BASE_DELAY_MS",
    );
    this.getTimeoutMs = parseNonNegativeInteger(
      getTimeoutMs,
      DEFAULT_R2_GET_TIMEOUT_MS,
      "RELEASE_STORAGE_GET_TIMEOUT_MS",
    );
    this.putTimeoutMs = parseNonNegativeInteger(
      putTimeoutMs,
      DEFAULT_R2_PUT_TIMEOUT_MS,
      "RELEASE_STORAGE_PUT_TIMEOUT_MS",
    );
  }

  async putFetch(url, init, normalizedObjectPath, attempt) {
    if (this.putTimeoutMs <= 0) {
      return this.fetchImpl(url, init);
    }
    const controller = new AbortController();
    const timer = setTimeout(() => {
      controller.abort();
    }, this.putTimeoutMs);
    try {
      return await this.fetchImpl(url, { ...init, signal: controller.signal });
    } catch (error) {
      if (error?.name === "AbortError") {
        throw new Error(
          `timed out uploading ${normalizedObjectPath} to R2 after ${this.putTimeoutMs}ms on attempt ${attempt}`,
        );
      }
      throw error;
    } finally {
      clearTimeout(timer);
    }
  }

  publicObjectUrl(objectPath) {
    return buildPublicObjectUrl(this.config, objectPath);
  }

  async getObjectBuffer(objectPath, { allowMissing = false } = {}) {
    return this.getR2ObjectBuffer(objectPath, { allowMissing });
  }

  async getFileObject({
    allowMissing = false,
    objectPath,
    outPath,
  }) {
    return this.getR2FileObject({ allowMissing, objectPath, outPath });
  }

  async getObjectText(objectPath, options = {}) {
    const bytes = await this.getObjectBuffer(objectPath, options);
    return bytes === null ? null : bytes.toString("utf8");
  }

  async getR2ObjectBuffer(objectPath, { allowMissing = false } = {}) {
    const signed = signR2Request(this.config, {
      method: "GET",
      objectPath,
    });
    const normalizedObjectPath = normalizeObjectPath(objectPath);
    const controller = this.getTimeoutMs > 0 ? new AbortController() : null;
    const timer = controller ? setTimeout(() => controller.abort(), this.getTimeoutMs) : null;
    try {
      const response = await this.fetchImpl(signed.url, {
        headers: signed.headers,
        method: "GET",
        ...(controller ? { signal: controller.signal } : {}),
      });
      if (response.ok) {
        return Buffer.from(await response.arrayBuffer());
      }
      const text = await responseText(response);
      if (allowMissing && isMissingObject(response.status, text)) {
        return null;
      }
      throw new Error(`failed to read ${normalizedObjectPath} from R2 (HTTP ${response.status}): ${text.slice(0, 500)}`);
    } catch (error) {
      if (error?.name === "AbortError") {
        throw new Error(`timed out downloading ${normalizedObjectPath} from R2 after ${this.getTimeoutMs}ms`);
      }
      throw error;
    } finally {
      if (timer) {
        clearTimeout(timer);
      }
    }
  }

  async getR2ObjectSha256(objectPath, { allowMissing = false } = {}) {
    const signed = signR2Request(this.config, {
      method: "GET",
      objectPath,
    });
    const normalizedObjectPath = normalizeObjectPath(objectPath);
    const controller = this.getTimeoutMs > 0 ? new AbortController() : null;
    const timer = controller ? setTimeout(() => controller.abort(), this.getTimeoutMs) : null;
    try {
      const response = await this.fetchImpl(signed.url, {
        headers: signed.headers,
        method: "GET",
        ...(controller ? { signal: controller.signal } : {}),
      });
      if (response.ok) {
        if (!response.body) {
          throw new Error(`failed to hash ${normalizedObjectPath} from R2: response body is empty`);
        }
        const hash = crypto.createHash("sha256");
        for await (const chunk of Readable.fromWeb(response.body)) {
          hash.update(chunk);
        }
        return hash.digest("hex");
      }
      const text = await responseText(response);
      if (allowMissing && isMissingObject(response.status, text)) {
        return null;
      }
      throw new Error(`failed to read ${normalizedObjectPath} from R2 (HTTP ${response.status}): ${text.slice(0, 500)}`);
    } catch (error) {
      if (error?.name === "AbortError") {
        throw new Error(`timed out downloading ${normalizedObjectPath} from R2 after ${this.getTimeoutMs}ms`);
      }
      throw error;
    } finally {
      if (timer) {
        clearTimeout(timer);
      }
    }
  }

  async getR2FileObject({ objectPath, outPath, allowMissing = false }) {
    const signed = signR2Request(this.config, {
      method: "GET",
      objectPath,
    });
    const normalizedObjectPath = normalizeObjectPath(objectPath);
    const controller = this.getTimeoutMs > 0 ? new AbortController() : null;
    const timer = controller ? setTimeout(() => controller.abort(), this.getTimeoutMs) : null;
    try {
      const response = await this.fetchImpl(signed.url, {
        headers: signed.headers,
        method: "GET",
        ...(controller ? { signal: controller.signal } : {}),
      });
      if (response.ok) {
        if (!response.body) {
          throw new Error(`failed to stream ${normalizedObjectPath} from R2: response body is empty`);
        }
        fs.mkdirSync(path.dirname(path.resolve(outPath)), { recursive: true });
        await pipeline(Readable.fromWeb(response.body), fs.createWriteStream(outPath));
        return { downloaded: true, objectPath: normalizedObjectPath, outPath };
      }
      const text = await responseText(response);
      if (allowMissing && isMissingObject(response.status, text)) {
        return null;
      }
      throw new Error(`failed to read ${normalizedObjectPath} from R2 (HTTP ${response.status}): ${text.slice(0, 500)}`);
    } catch (error) {
      if (error?.name === "AbortError") {
        throw new Error(`timed out downloading ${normalizedObjectPath} from R2 after ${this.getTimeoutMs}ms`);
      }
      throw error;
    } finally {
      if (timer) {
        clearTimeout(timer);
      }
    }
  }

  async putObject({
    body,
    contentType = "application/octet-stream",
    objectPath,
    upsert = false,
    verifyExisting = false,
  }) {
    const bytes = Buffer.isBuffer(body) ? body : Buffer.from(String(body));
    return this.putR2Object({ body: bytes, contentType, objectPath, upsert, verifyExisting });
  }

  async putFileObject({
    contentType = "application/octet-stream",
    filePath,
    objectPath,
    upsert = false,
    verifyExisting = false,
  }) {
    const stats = fs.statSync(filePath);
    if (!stats.isFile()) {
      throw new Error(`release storage source is not a file: ${filePath}`);
    }
    const payloadHash = await sha256FileHex(filePath);
    return this.putR2FileObject({
      contentLength: stats.size,
      contentType,
      filePath,
      objectPath,
      payloadHash,
      upsert,
      verifyExisting,
    });
  }

  async putR2Object({ body, contentType, objectPath, upsert, verifyExisting }) {
    const headers = {
      "content-type": contentType,
      ...(upsert ? {} : { "if-none-match": "*" }),
    };
    const normalizedObjectPath = normalizeObjectPath(objectPath);
    for (let attempt = 1; attempt <= this.putRetryAttempts; attempt += 1) {
      const signed = signR2Request(this.config, {
        body,
        headers,
        method: "PUT",
        objectPath,
      });
      let response;
      try {
        response = await this.putFetch(signed.url, {
          body: signed.body,
          headers: signed.headers,
          method: "PUT",
        }, normalizedObjectPath, attempt);
      } catch (error) {
        if (attempt >= this.putRetryAttempts) {
          throw error;
        }
        console.error(`warn: R2 upload request failed for ${normalizedObjectPath} (attempt ${attempt}/${this.putRetryAttempts}); retrying`);
        await sleep(attempt * this.putRetryBaseDelayMs);
        continue;
      }
      if (response.ok) {
        return { existing: false, objectPath: normalizedObjectPath, uploaded: true };
      }
      const text = await responseText(response);
      if (!upsert && verifyExisting && (response.status === 409 || response.status === 412)) {
        await this.verifyExistingObject(objectPath, body);
        return { existing: true, objectPath: normalizedObjectPath, uploaded: false };
      }
      const error = new Error(`failed to upload ${normalizedObjectPath} to R2 (HTTP ${response.status}): ${text.slice(0, 500)}`);
      if (!isRetryableR2Status(response.status) || attempt >= this.putRetryAttempts) {
        throw error;
      }
      console.error(`warn: transient R2 upload failure for ${normalizedObjectPath} (HTTP ${response.status}, attempt ${attempt}/${this.putRetryAttempts}); retrying`);
      await sleep(attempt * this.putRetryBaseDelayMs);
    }
    throw new Error(`failed to upload ${normalizedObjectPath} to R2 after ${this.putRetryAttempts} attempts`);
  }

  async putR2FileObject({ contentLength, contentType, filePath, objectPath, payloadHash, upsert, verifyExisting }) {
    const headers = {
      "content-length": String(contentLength),
      "content-type": contentType,
      ...(upsert ? {} : { "if-none-match": "*" }),
    };
    const normalizedObjectPath = normalizeObjectPath(objectPath);
    for (let attempt = 1; attempt <= this.putRetryAttempts; attempt += 1) {
      const signed = signR2Request(this.config, {
        headers,
        method: "PUT",
        objectPath,
        payloadHash,
      });
      let response;
      try {
        response = await this.putFetch(signed.url, {
          body: fs.createReadStream(filePath),
          duplex: "half",
          headers: signed.headers,
          method: "PUT",
        }, normalizedObjectPath, attempt);
      } catch (error) {
        if (attempt >= this.putRetryAttempts) {
          throw error;
        }
        console.error(`warn: R2 upload request failed for ${normalizedObjectPath} (attempt ${attempt}/${this.putRetryAttempts}); retrying`);
        await sleep(attempt * this.putRetryBaseDelayMs);
        continue;
      }
      if (response.ok) {
        return { existing: false, objectPath: normalizedObjectPath, uploaded: true };
      }
      const text = await responseText(response);
      if (!upsert && verifyExisting && (response.status === 409 || response.status === 412)) {
        await this.verifyExistingObjectHash(objectPath, payloadHash);
        return { existing: true, objectPath: normalizedObjectPath, uploaded: false };
      }
      const error = new Error(`failed to upload ${normalizedObjectPath} to R2 (HTTP ${response.status}): ${text.slice(0, 500)}`);
      if (!isRetryableR2Status(response.status) || attempt >= this.putRetryAttempts) {
        throw error;
      }
      console.error(`warn: transient R2 upload failure for ${normalizedObjectPath} (HTTP ${response.status}, attempt ${attempt}/${this.putRetryAttempts}); retrying`);
      await sleep(attempt * this.putRetryBaseDelayMs);
    }
    throw new Error(`failed to upload ${normalizedObjectPath} to R2 after ${this.putRetryAttempts} attempts`);
  }

  async verifyExistingObject(objectPath, expectedBytes) {
    const existing = await this.getObjectBuffer(objectPath);
    const expectedSha = sha256Hex(expectedBytes);
    await this.verifyExistingObjectHash(objectPath, expectedSha, existing);
  }

  async verifyExistingObjectHash(objectPath, expectedSha, existingBytes = null) {
    const existingSha = existingBytes === null
      ? await this.getR2ObjectSha256(objectPath)
      : sha256Hex(existingBytes);
    if (expectedSha !== existingSha) {
      throw new Error(
        `immutable upload conflict for ${normalizeObjectPath(objectPath)} has mismatched existing content (local sha256 ${expectedSha}, existing sha256 ${existingSha})`,
      );
    }
  }

  async deleteObject(objectPath) {
    return this.deleteR2Object(objectPath);
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
    void fileSizeLimitBytes;
    return { bucket: this.config.bucket, provider: "r2", skipped: true };
  }
}

function readFileBytes(filePath) {
  return fs.readFileSync(filePath);
}

module.exports = {
  DEFAULT_PUBLIC_STORAGE_ORIGIN,
  DEFAULT_PUBLIC_STORAGE_BUCKET,
  DEFAULT_R2_REGION,
  DEFAULT_R2_GET_TIMEOUT_MS,
  DEFAULT_R2_PUT_TIMEOUT_MS,
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
  sha256FileHex,
  sha256Hex,
  signR2Request,
};
