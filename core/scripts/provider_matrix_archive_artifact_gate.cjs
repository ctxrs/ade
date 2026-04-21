#!/usr/bin/env node

const crypto = require("node:crypto");
const fs = require("node:fs");
const path = require("node:path");
const { Readable } = require("node:stream");
const { pathToFileURL, fileURLToPath } = require("node:url");

const DEFAULT_TIMEOUT_MS = 5 * 60 * 1000;
const DEFAULT_FETCH_MAX_ATTEMPTS = 3;
const DEFAULT_FETCH_RETRY_DELAY_MS = 1000;
const RETRYABLE_HTTP_STATUSES = new Set([408, 429, 500, 502, 503, 504]);
const coreRoot = path.resolve(__dirname, "..");
const defaultMatrixPath = path.join(
  coreRoot,
  "crates",
  "ctx-provider-accounts",
  "src",
  "provider_matrix.json",
);

function fail(message) {
  console.error(`error: ${message}`);
  process.exit(1);
}

function parseArgs(argv) {
  const out = {
    matrixPath: String(process.env.CTX_BUNDLE_MATRIX_JSON || "").trim()
      ? path.resolve(process.env.CTX_BUNDLE_MATRIX_JSON)
      : defaultMatrixPath,
    providers: [],
    targets: [],
    timeoutMs: DEFAULT_TIMEOUT_MS,
  };
  for (let i = 2; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--matrix") {
      out.matrixPath = argv[++i] || "";
      continue;
    }
    if (arg === "--provider") {
      out.providers.push((argv[++i] || "").trim());
      continue;
    }
    if (arg === "--target") {
      out.targets.push((argv[++i] || "").trim());
      continue;
    }
    if (arg === "--timeout-ms") {
      const raw = Number(argv[++i] || "");
      if (!Number.isFinite(raw) || raw <= 0) {
        fail(`invalid --timeout-ms value: ${argv[i]}`);
      }
      out.timeoutMs = Math.floor(raw);
      continue;
    }
    if (arg === "--help" || arg === "-h") {
      console.log(
        "Usage: node core/scripts/provider_matrix_archive_artifact_gate.cjs [--matrix <provider_matrix.json>] [--provider <id>]... [--target <os-arch>]... [--timeout-ms <ms>]",
      );
      process.exit(0);
    }
    fail(`unknown argument: ${arg}`);
  }
  out.providers = out.providers.filter(Boolean);
  out.targets = out.targets.filter(Boolean);
  return out;
}

function readMatrix(matrixPath) {
  const resolved = path.resolve(matrixPath);
  if (!fs.existsSync(resolved)) {
    fail(`matrix file does not exist: ${resolved}`);
  }
  return JSON.parse(fs.readFileSync(resolved, "utf8"));
}

function collectManagedArchiveTargets(matrix, providerFilter = [], targetFilter = []) {
  const requested = new Set(providerFilter);
  const requestedTargets = new Set(targetFilter);
  const matchedProviders = new Set();
  const targets = [];

  for (const entry of matrix.providers || []) {
    if (requested.size > 0 && !requested.has(entry.id)) {
      continue;
    }
    matchedProviders.add(entry.id);
    const install = entry.managed_install;
    if (!install || install.kind !== "archive") {
      continue;
    }
    const targetMap = install.targets || {};
    for (const [targetKey, target] of Object.entries(targetMap)) {
      if (requestedTargets.size > 0 && !requestedTargets.has(targetKey)) {
        continue;
      }
      targets.push({
        providerId: entry.id,
        targetKey,
        url: String(target?.url || "").trim(),
        expectedSha256: String(target?.sha256 || "").trim().toLowerCase(),
        sizeBytes: Number.isFinite(target?.size_bytes) ? Number(target.size_bytes) : null,
      });
    }
  }

  const missing = [...requested].filter((id) => !matchedProviders.has(id));
  return { missing, targets };
}

class HttpRequestError extends Error {
  constructor(status, statusText) {
    super(`request failed with ${status} ${statusText}`);
    this.name = "HttpRequestError";
    this.status = status;
  }
}

async function sha256Readable(readable) {
  const hash = crypto.createHash("sha256");
  let sizeBytes = 0;
  for await (const chunk of readable) {
    hash.update(chunk);
    sizeBytes += chunk.length;
  }
  return {
    sha256: hash.digest("hex"),
    sizeBytes,
  };
}

async function fetchDigest(url, timeoutMs) {
  const parsed = new URL(url);
  if (parsed.protocol === "file:") {
    return sha256Readable(fs.createReadStream(fileURLToPath(parsed)));
  }
  if (parsed.protocol !== "http:" && parsed.protocol !== "https:") {
    throw new Error(`unsupported URL scheme: ${parsed.protocol}`);
  }
  const response = await fetch(parsed, {
    redirect: "follow",
    signal: AbortSignal.timeout(timeoutMs),
  });
  if (!response.ok) {
    throw new HttpRequestError(response.status, response.statusText);
  }
  if (!response.body) {
    throw new Error("response body missing");
  }
  return sha256Readable(Readable.fromWeb(response.body));
}

function isRetryableFetchError(error) {
  const message = String(error?.message || "");
  return (
    error &&
    typeof error === "object" &&
    ((Number.isInteger(error.status) &&
      RETRYABLE_HTTP_STATUSES.has(error.status)) ||
      error.name === "AbortError" ||
      error.name === "TimeoutError" ||
      /\b(fetch failed|ECONNRESET|ETIMEDOUT|EAI_AGAIN|ENOTFOUND)\b/i.test(message))
  );
}

function isMissingArtifactHttpError(error, url = "") {
  if (!(error instanceof HttpRequestError)) {
    return false;
  }
  if (error.status === 404) {
    return true;
  }
  if (error.status !== 400) {
    return false;
  }
  try {
    const parsed = new URL(String(url || ""));
    return /\/storage\/v1\/object\/public\//.test(parsed.pathname);
  } catch {
    return false;
  }
}

async function sleep(ms) {
  if (ms <= 0) {
    return;
  }
  await new Promise((resolve) => setTimeout(resolve, ms));
}

async function verifyManagedArchiveTargets(
  targets,
  {
    timeoutMs = DEFAULT_TIMEOUT_MS,
    maxAttempts = DEFAULT_FETCH_MAX_ATTEMPTS,
    retryDelayMs = DEFAULT_FETCH_RETRY_DELAY_MS,
  } = {},
) {
  const results = await verifyManagedArchiveTargetsDetailed(targets, {
    timeoutMs,
    maxAttempts,
    retryDelayMs,
  });
  return {
    errors: results.filter((result) => result.status !== "verified").map((result) => result.message),
    verifiedCount: results.filter((result) => result.status === "verified").length,
  };
}

async function verifyManagedArchiveTargetsDetailed(
  targets,
  {
    timeoutMs = DEFAULT_TIMEOUT_MS,
    maxAttempts = DEFAULT_FETCH_MAX_ATTEMPTS,
    retryDelayMs = DEFAULT_FETCH_RETRY_DELAY_MS,
  } = {},
) {
  const results = [];
  for (const target of targets) {
    if (!target.url) {
      results.push({
        providerId: target.providerId,
        targetKey: target.targetKey,
        status: "error",
        errorKind: "metadata",
        message: `provider=${target.providerId} target=${target.targetKey}: missing url`,
      });
      continue;
    }
    if (!target.expectedSha256) {
      results.push({
        providerId: target.providerId,
        targetKey: target.targetKey,
        status: "error",
        errorKind: "metadata",
        message: `provider=${target.providerId} target=${target.targetKey}: missing expected sha256`,
      });
      continue;
    }
    try {
      let result = null;
      let attempt = 0;
      let nextDelayMs = retryDelayMs;
      while (attempt < maxAttempts) {
        attempt += 1;
        try {
          result = await fetchDigest(target.url, timeoutMs);
          break;
        } catch (error) {
          if (!isRetryableFetchError(error) || attempt >= maxAttempts) {
            throw error;
          }
          console.error(
            `warn: provider=${target.providerId} target=${target.targetKey}: transient fetch failure (${error.message}); retrying (${attempt}/${maxAttempts}) in ${nextDelayMs}ms`,
          );
          await sleep(nextDelayMs);
          nextDelayMs *= 2;
        }
      }
      if (!result) {
        throw new Error("internal: archive fetch produced no result");
      }
      if (result.sha256 !== target.expectedSha256) {
        results.push({
          providerId: target.providerId,
          targetKey: target.targetKey,
          status: "error",
          errorKind: "checksum",
          message:
            `provider=${target.providerId} target=${target.targetKey}: checksum mismatch expected=${target.expectedSha256} actual=${result.sha256}`,
        });
        continue;
      }
      if (target.sizeBytes !== null && result.sizeBytes !== target.sizeBytes) {
        results.push({
          providerId: target.providerId,
          targetKey: target.targetKey,
          status: "error",
          errorKind: "size",
          message:
            `provider=${target.providerId} target=${target.targetKey}: size mismatch expected=${target.sizeBytes} actual=${result.sizeBytes}`,
        });
        continue;
      }
      results.push({
        providerId: target.providerId,
        targetKey: target.targetKey,
        status: "verified",
        errorKind: "",
        message: "",
        sha256: result.sha256,
        sizeBytes: result.sizeBytes,
      });
    } catch (error) {
      const errorMessage = error?.message ?? String(error);
      const missingArtifact = isMissingArtifactHttpError(error, target.url);
      results.push({
        providerId: target.providerId,
        targetKey: target.targetKey,
        status: missingArtifact ? "missing" : "error",
        errorKind: missingArtifact ? "missing" : "fetch",
        message: `provider=${target.providerId} target=${target.targetKey}: ${errorMessage}`,
      });
    }
  }
  return results;
}

async function main(argv = process.argv) {
  const args = parseArgs(argv);
  const matrix = readMatrix(args.matrixPath);
  const { missing, targets } = collectManagedArchiveTargets(matrix, args.providers, args.targets);
  if (missing.length > 0) {
    fail(`requested provider ids not present in matrix: ${missing.join(", ")}`);
  }
  if (targets.length === 0) {
    fail("no managed archive targets matched the selected provider set");
  }

  const result = await verifyManagedArchiveTargets(targets, {
    timeoutMs: args.timeoutMs,
  });
  if (result.errors.length > 0) {
    for (const error of result.errors) {
      console.error(`error: ${error}`);
    }
    process.exit(1);
  }

  const providerCount = new Set(targets.map((target) => target.providerId)).size;
  console.log(
    `ok: verified ${result.verifiedCount} managed archive targets across ${providerCount} provider(s)`,
  );
}

if (require.main === module) {
  main().catch((error) => {
    fail(error?.message ?? String(error));
  });
}

module.exports = {
  collectManagedArchiveTargets,
  defaultMatrixPath,
  fetchDigest,
  HttpRequestError,
  isMissingArtifactHttpError,
  isRetryableFetchError,
  parseArgs,
  pathToFileURL,
  readMatrix,
  verifyManagedArchiveTargetsDetailed,
  verifyManagedArchiveTargets,
};
