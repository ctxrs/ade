#!/usr/bin/env node
import process from "node:process";
import {
  DEFAULT_LOCK_PATH,
  DEFAULT_RELEASE_STORAGE_BUCKET,
  ensureEntrySchema,
  fetchBufferWithRetry,
  loadLock,
  lockEntryLabel,
  mirrorRelativePathFor,
  mirrorUrlFor,
  normalizePlatform,
  resolveMirrorBaseUrl,
  selectEntries,
  sha256Hex,
  upstreamUrlFor,
} from "./lib/tauri_tools_lock.mjs";
import { buildStorageClientFromEnv } from "../core/scripts/lib/release_storage.cjs";

const printUsage = () => {
  console.log(`Usage: node scripts/tauri_tools_mirror_sync.mjs [options]

Options:
  --lock <path>      Lock file path (default: ${DEFAULT_LOCK_PATH})
  --platform <id>    Optional platform filter (linux-x64|linux-arm64)
  --check-only       Verify mirror assets exist with expected content-length
  --help             Show this help

Required env:
  RELEASE_STORAGE_PROVIDER=r2
  RELEASE_STORAGE_BUCKET or CTX_RELEASES_R2_BUCKET
  RELEASE_R2_ENDPOINT or RELEASE_R2_ACCOUNT_ID
  RELEASE_R2_ACCESS_KEY_ID
  RELEASE_R2_SECRET_ACCESS_KEY
  RELEASE_PUBLIC_STORAGE_BUCKET (optional, default: ${DEFAULT_RELEASE_STORAGE_BUCKET})
`);
};

const args = process.argv.slice(2);
let lockPath = DEFAULT_LOCK_PATH;
let platform = "";
let checkOnly = false;

for (let i = 0; i < args.length; i += 1) {
  const arg = args[i];
  if (arg === "--lock") {
    lockPath = String(args[i + 1] || "").trim();
    i += 1;
    continue;
  }
  if (arg === "--platform") {
    platform = normalizePlatform(args[i + 1]);
    i += 1;
    continue;
  }
  if (arg === "--check-only") {
    checkOnly = true;
    continue;
  }
  if (arg === "--help" || arg === "-h") {
    printUsage();
    process.exit(0);
  }
  throw new Error(`unknown argument: ${arg}`);
}

const storageClient = buildStorageClientFromEnv(process.env);
const mirrorBaseUrl = resolveMirrorBaseUrl();

const { lock } = loadLock(lockPath);
const selected = selectEntries(lock.entries, platform);
if (selected.length === 0) {
  throw new Error(`no tauri tool lock entries selected for platform='${platform || "all"}'`);
}

for (const entry of selected) {
  ensureEntrySchema(entry);
  const label = lockEntryLabel(entry);
  const expectedSize = Number(entry.size_bytes || 0);
  const expectedSha = String(entry.sha256 || "").trim().toLowerCase();
  if (!expectedSize || !expectedSha) {
    throw new Error(`lock entry missing checksum/size: ${entry.id}`);
  }
  const mirrorUrl = mirrorUrlFor(mirrorBaseUrl, entry);
  const mirrorPath = mirrorRelativePathFor(entry);
  process.stdout.write(`mirror sync ${label}\n`);

  const headResp = await fetch(mirrorUrl, { method: "HEAD", redirect: "follow" });
  const mirrorSize = Number(headResp.headers.get("content-length") || 0);
  const hasMirrorAsset = headResp.ok && mirrorSize === expectedSize;
  if (checkOnly) {
    if (!hasMirrorAsset) {
      throw new Error(`mirror missing or size mismatch for ${entry.id}: ${mirrorUrl}`);
    }
    continue;
  }
  if (!hasMirrorAsset) {
    const upstream = upstreamUrlFor(entry);
    process.stdout.write(`  download upstream ${upstream}\n`);
    const payload = await fetchBufferWithRetry(upstream);
    const actualSha = sha256Hex(payload).toLowerCase();
    if (actualSha !== expectedSha) {
      throw new Error(`sha mismatch for ${entry.id}: lock=${expectedSha} actual=${actualSha}`);
    }
    if (payload.length !== expectedSize) {
      throw new Error(`size mismatch for ${entry.id}: lock=${expectedSize} actual=${payload.length}`);
    }
    await storageClient.putObject({
      body: payload,
      contentType: "application/octet-stream",
      objectPath: mirrorPath,
      upsert: true,
    });
  }

  const verifyPayload = await fetchBufferWithRetry(mirrorUrl, 3);
  const verifySha = sha256Hex(verifyPayload).toLowerCase();
  if (verifySha !== expectedSha) {
    throw new Error(`mirror sha mismatch for ${entry.id}: expected=${expectedSha} got=${verifySha}`);
  }
}

process.stdout.write(`ok: mirror sync complete (${selected.length} entries)\n`);
