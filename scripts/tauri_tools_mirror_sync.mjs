#!/usr/bin/env node
import process from "node:process";
import {
  DEFAULT_LOCK_PATH,
  DEFAULT_SUPABASE_STORAGE_BUCKET,
  ensureEntrySchema,
  fetchBufferWithRetry,
  loadLock,
  lockEntryLabel,
  mirrorRelativePathFor,
  mirrorUrlFor,
  normalizePlatform,
  resolveMirrorBaseUrl,
  resolveSupabaseStorageBucket,
  selectEntries,
  sha256Hex,
  upstreamUrlFor,
} from "./lib/tauri_tools_lock.mjs";

const printUsage = () => {
  console.log(`Usage: node scripts/tauri_tools_mirror_sync.mjs [options]

Options:
  --lock <path>      Lock file path (default: ${DEFAULT_LOCK_PATH})
  --platform <id>    Optional platform filter (linux-x64|linux-arm64)
  --check-only       Verify mirror assets exist with expected content-length
  --help             Show this help

Required env:
  SUPABASE_URL
  SUPABASE_SERVICE_ROLE_KEY
  SUPABASE_STORAGE_BUCKET (optional, default: ${DEFAULT_SUPABASE_STORAGE_BUCKET})
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

const supabaseUrl = String(process.env.SUPABASE_URL || "").trim().replace(/\/+$/, "");
const serviceRoleKey = String(process.env.SUPABASE_SERVICE_ROLE_KEY || "").trim();
const storageBucket = resolveSupabaseStorageBucket();
if (!supabaseUrl || !serviceRoleKey) {
  throw new Error("missing required env (SUPABASE_URL, SUPABASE_SERVICE_ROLE_KEY)");
}

const mirrorBaseUrl = resolveMirrorBaseUrl();
const uploadBaseUrl = `${supabaseUrl}/storage/v1/object/${storageBucket}`;

const storageHeaders = {
  authorization: `Bearer ${serviceRoleKey}`,
  apikey: serviceRoleKey,
  "x-upsert": "true",
};

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
    const uploadUrl = `${uploadBaseUrl}/${mirrorPath}`;
    const uploadResp = await fetch(uploadUrl, {
      method: "POST",
      headers: {
        ...storageHeaders,
        "content-type": "application/octet-stream",
      },
      body: payload,
    });
    if (!uploadResp.ok) {
      const text = await uploadResp.text();
      throw new Error(`failed to upload ${entry.id} to mirror: HTTP ${uploadResp.status} ${text}`);
    }
  }

  const verifyPayload = await fetchBufferWithRetry(mirrorUrl, 3);
  const verifySha = sha256Hex(verifyPayload).toLowerCase();
  if (verifySha !== expectedSha) {
    throw new Error(`mirror sha mismatch for ${entry.id}: expected=${expectedSha} got=${verifySha}`);
  }
}

process.stdout.write(`ok: mirror sync complete (${selected.length} entries)\n`);
