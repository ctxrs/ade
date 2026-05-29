#!/usr/bin/env node
"use strict";

const fs = require("node:fs");
const path = require("node:path");

const REPO_ROOT = path.resolve(__dirname, "..", "..");
const DEFAULT_RUNTIME_LOCK_PATH = path.join(
  REPO_ROOT,
  "core/apps/desktop/src-tauri/bundles/runtime_lock.v2.json",
);
const DEFAULT_TAURI_TOOLS_LOCK_PATH = path.join(
  REPO_ROOT,
  "core/apps/desktop/src-tauri/bundles/tauri_tools_lock.v1.json",
);
const PUBLIC_STORAGE_PREFIX = "https://api.ctx.rs/storage/v1/object/public/releases/";
const PUBLIC_STORAGE_PATH_PREFIX = "/storage/v1/object/public/releases/";

function readJson(filePath) {
  return JSON.parse(fs.readFileSync(filePath, "utf8"));
}

function encodeObjectPath(objectPath) {
  return objectPath.split("/").map((segment) => encodeURIComponent(segment)).join("/");
}

function publicUrlForObjectPath(objectPath) {
  return `${PUBLIC_STORAGE_PREFIX}${encodeObjectPath(objectPath)}`;
}

function objectPathFromPublicStorageUrl(value) {
  const raw = String(value || "").trim();
  if (!raw.startsWith(PUBLIC_STORAGE_PREFIX)) return "";
  const url = new URL(raw);
  if (url.protocol !== "https:" || url.host !== "api.ctx.rs") return "";
  if (!url.pathname.startsWith(PUBLIC_STORAGE_PATH_PREFIX)) return "";
  return url.pathname
    .slice(PUBLIC_STORAGE_PATH_PREFIX.length)
    .split("/")
    .map((segment) => decodeURIComponent(segment))
    .join("/");
}

function collectRuntimeStorageObjects(lockPath = DEFAULT_RUNTIME_LOCK_PATH) {
  const lock = readJson(lockPath);
  const entries = [];
  const visit = (value) => {
    if (Array.isArray(value)) {
      for (const item of value) visit(item);
      return;
    }
    if (!value || typeof value !== "object") return;
    for (const [key, child] of Object.entries(value)) {
      if (key === "uri" && typeof child === "string") {
        const objectPath = objectPathFromPublicStorageUrl(child);
        if (objectPath) {
          entries.push({
            objectPath,
            source: "runtime_lock.v2.json",
            url: publicUrlForObjectPath(objectPath),
          });
        }
        continue;
      }
      visit(child);
    }
  };
  visit(lock);
  return entries;
}

function collectTauriToolStorageObjects(lockPath = DEFAULT_TAURI_TOOLS_LOCK_PATH) {
  const lock = readJson(lockPath);
  if (lock.schema_version !== 1 || !Array.isArray(lock.entries)) {
    throw new Error(`invalid tauri tools lock schema: ${lockPath}`);
  }
  return lock.entries.map((entry) => {
    for (const key of ["owner", "repo", "tag", "asset"]) {
      if (!entry || typeof entry[key] !== "string" || !entry[key].trim()) {
        throw new Error(`invalid tauri tools lock entry missing ${key}: ${JSON.stringify(entry)}`);
      }
    }
    const objectPath = `tauri-tools/${entry.owner}/${entry.repo}/releases/download/${entry.tag}/${entry.asset}`;
    const expectedSizeBytes = Number(entry.size_bytes);
    if (!Number.isSafeInteger(expectedSizeBytes) || expectedSizeBytes < 0) {
      throw new Error(`invalid tauri tools lock entry size_bytes for ${entry.id || objectPath}`);
    }
    return {
      expectedSizeBytes,
      objectPath,
      source: "tauri_tools_lock.v1.json",
      url: publicUrlForObjectPath(objectPath),
    };
  });
}

function collectLockedStorageObjects(options = {}) {
  const entries = [
    ...collectRuntimeStorageObjects(options.runtimeLockPath),
    ...collectTauriToolStorageObjects(options.tauriToolsLockPath),
  ];
  const byPath = new Map();
  for (const entry of entries) {
    if (!entry.objectPath || entry.objectPath.includes("..") || entry.objectPath.startsWith("/")) {
      throw new Error(`unsafe release storage object path: ${entry.objectPath || "<empty>"}`);
    }
    if (!byPath.has(entry.objectPath)) {
      byPath.set(entry.objectPath, entry);
    }
  }
  return [...byPath.values()].sort((a, b) => a.objectPath.localeCompare(b.objectPath));
}

async function checkLockedStorageObjects(options = {}) {
  const entries = collectLockedStorageObjects(options);
  const fetchImpl = options.fetchImpl || fetch;
  const failures = [];
  for (const entry of entries) {
    let response;
    try {
      response = await fetchImpl(entry.url, { method: "HEAD", redirect: "follow" });
    } catch (error) {
      failures.push(`${entry.objectPath}: ${error instanceof Error ? error.message : String(error)}`);
      continue;
    }
    if (!response || !response.ok) {
      failures.push(`${entry.objectPath}: HTTP ${response ? response.status : "000"}`);
      continue;
    }
    if (entry.expectedSizeBytes !== undefined && entry.expectedSizeBytes !== null) {
      const contentLength = Number(response.headers && response.headers.get
        ? response.headers.get("content-length")
        : 0);
      if (!Number.isFinite(contentLength) || contentLength !== entry.expectedSizeBytes) {
        failures.push(`${entry.objectPath}: size mismatch expected ${entry.expectedSizeBytes} got ${contentLength}`);
      }
    }
  }
  return { entries, failures };
}

function parseArgs(argv) {
  const options = {
    checkPresence: false,
    json: false,
  };
  for (const arg of argv) {
    if (arg === "--check-presence") {
      options.checkPresence = true;
      continue;
    }
    if (arg === "--json") {
      options.json = true;
      continue;
    }
    if (arg === "--help" || arg === "-h") {
      options.help = true;
      continue;
    }
    throw new Error(`unsupported argument: ${arg}`);
  }
  return options;
}

function printUsage() {
  console.log(`usage: node core/scripts/release_public_storage_locked_objects.cjs [--check-presence] [--json]

Lists public release-storage objects referenced by lockfiles. With
--check-presence, each object must be readable through
https://api.ctx.rs/storage/v1/object/public/releases/...`);
}

async function main() {
  const options = parseArgs(process.argv.slice(2));
  if (options.help) {
    printUsage();
    return;
  }
  if (options.checkPresence) {
    const result = await checkLockedStorageObjects();
    if (options.json) {
      console.log(JSON.stringify(result, null, 2));
    } else {
      for (const entry of result.entries) {
        console.log(`ok: ${entry.objectPath}`);
      }
    }
    if (result.failures.length > 0) {
      for (const failure of result.failures) {
        console.error(`error: ${failure}`);
      }
      process.exit(1);
    }
    return;
  }
  const entries = collectLockedStorageObjects();
  if (options.json) {
    console.log(JSON.stringify({ entries }, null, 2));
    return;
  }
  for (const entry of entries) {
    console.log(entry.url);
  }
}

if (require.main === module) {
  main().catch((error) => {
    console.error(error && error.stack ? error.stack : String(error));
    process.exit(1);
  });
}

module.exports = {
  checkLockedStorageObjects,
  collectLockedStorageObjects,
  collectRuntimeStorageObjects,
  collectTauriToolStorageObjects,
  objectPathFromPublicStorageUrl,
  publicUrlForObjectPath,
};
