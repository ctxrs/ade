#!/usr/bin/env node
import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import {
  DEFAULT_LOCK_PATH,
  ensureEntrySchema,
  fetchBufferWithRetry,
  loadLock,
  lockEntryLabel,
  mirrorUrlFor,
  normalizePlatform,
  resolveMirrorBaseUrl,
  selectEntries,
  sha256Hex,
} from "./lib/tauri_tools_lock.mjs";

const printUsage = () => {
  console.log(`Usage: node scripts/tauri_tools_prefetch.mjs --platform <id> [options]

Options:
  --platform <id>    Required platform (linux-x64|linux-arm64)
  --lock <path>      Lock file path (default: ${DEFAULT_LOCK_PATH})
  --cache-dir <dir>  Optional cache dir override (default: $XDG_CACHE_HOME/tauri or $HOME/.cache/tauri)
  --check-only       Download/verify mirror bytes only; do not write cache
  --help             Show this help

Required env:
  TAURI_TOOLS_MIRROR_BASE_URL
or
  SUPABASE_URL (+ optional SUPABASE_STORAGE_BUCKET, default: releases)
`);
};

const args = process.argv.slice(2);
let lockPath = DEFAULT_LOCK_PATH;
let platform = "";
let cacheDirArg = "";
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
  if (arg === "--cache-dir") {
    cacheDirArg = String(args[i + 1] || "").trim();
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

if (!platform) {
  throw new Error("missing required --platform");
}

const mirrorBaseUrl = resolveMirrorBaseUrl();
if (!mirrorBaseUrl) {
  throw new Error(
    "missing mirror base URL: set TAURI_TOOLS_MIRROR_BASE_URL or SUPABASE_URL (optional SUPABASE_STORAGE_BUCKET)",
  );
}

const fallbackCacheDir = process.env.XDG_CACHE_HOME
  ? path.join(process.env.XDG_CACHE_HOME, "tauri")
  : path.join(String(process.env.HOME || ""), ".cache", "tauri");
const cacheDir = path.resolve(cacheDirArg || fallbackCacheDir);

const { lock } = loadLock(lockPath);
const selected = selectEntries(lock.entries, platform);
if (selected.length === 0) {
  throw new Error(`no tauri tool lock entries selected for platform='${platform}'`);
}

if (!checkOnly) {
  fs.mkdirSync(cacheDir, { recursive: true });
}

for (const entry of selected) {
  ensureEntrySchema(entry);
  const expectedSha = String(entry.sha256 || "").trim().toLowerCase();
  const expectedSize = Number(entry.size_bytes || 0);
  if (!expectedSha || !expectedSize) {
    throw new Error(`lock entry missing checksum/size: ${entry.id}`);
  }
  const mirrorUrl = mirrorUrlFor(mirrorBaseUrl, entry);
  process.stdout.write(`prefetch ${lockEntryLabel(entry)} <- ${mirrorUrl}\n`);
  const payload = await fetchBufferWithRetry(mirrorUrl);
  const actualSha = sha256Hex(payload).toLowerCase();
  if (actualSha !== expectedSha) {
    throw new Error(`sha mismatch for ${entry.id}: expected=${expectedSha} got=${actualSha}`);
  }
  if (payload.length !== expectedSize) {
    throw new Error(`size mismatch for ${entry.id}: expected=${expectedSize} got=${payload.length}`);
  }
  if (checkOnly) continue;
  const destination = path.join(cacheDir, entry.cache_filename);
  fs.writeFileSync(destination, payload);
  fs.chmodSync(destination, 0o755);
}

process.stdout.write(
  `${checkOnly ? "ok: verified" : "ok: prefetched"} ${selected.length} tauri tool assets for ${platform}\n`,
);
