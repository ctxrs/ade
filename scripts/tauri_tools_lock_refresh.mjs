#!/usr/bin/env node
import process from "node:process";
import {
  DEFAULT_LOCK_PATH,
  ensureEntrySchema,
  fetchBufferWithRetry,
  loadLock,
  lockEntryLabel,
  normalizePlatform,
  selectEntries,
  sha256Hex,
  upstreamUrlFor,
  writeLock,
} from "./lib/tauri_tools_lock.mjs";

const printUsage = () => {
  console.log(`Usage: node scripts/tauri_tools_lock_refresh.mjs [options]

Options:
  --lock <path>      Lock file path (default: ${DEFAULT_LOCK_PATH})
  --platform <id>    Optional platform filter (linux-x64|linux-arm64)
  --check            Validate lock checksums/sizes without writing
  --help             Show this help
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
  if (arg === "--check") {
    checkOnly = true;
    continue;
  }
  if (arg === "--help" || arg === "-h") {
    printUsage();
    process.exit(0);
  }
  throw new Error(`unknown argument: ${arg}`);
}

const { lockPath: resolvedLockPath, lock } = loadLock(lockPath);
const selected = selectEntries(lock.entries, platform);
if (selected.length === 0) {
  throw new Error(`no tauri tool lock entries selected for platform='${platform || "all"}'`);
}

let updated = false;
for (const entry of selected) {
  ensureEntrySchema(entry);
  const upstream = upstreamUrlFor(entry);
  process.stdout.write(`refresh ${lockEntryLabel(entry)} -> ${upstream}\n`);
  const payload = await fetchBufferWithRetry(upstream);
  const nextSha = sha256Hex(payload);
  const nextSize = payload.length;
  if (checkOnly) {
    if (String(entry.sha256 || "").trim().toLowerCase() !== nextSha.toLowerCase()) {
      throw new Error(
        `sha mismatch for ${entry.id}: lock=${String(entry.sha256 || "").trim()} actual=${nextSha}`,
      );
    }
    if (Number(entry.size_bytes || 0) !== nextSize) {
      throw new Error(
        `size mismatch for ${entry.id}: lock=${Number(entry.size_bytes || 0)} actual=${nextSize}`,
      );
    }
    continue;
  }
  if (String(entry.sha256 || "").trim().toLowerCase() !== nextSha.toLowerCase()) {
    entry.sha256 = nextSha;
    updated = true;
  }
  if (Number(entry.size_bytes || 0) !== nextSize) {
    entry.size_bytes = nextSize;
    updated = true;
  }
}

if (checkOnly) {
  process.stdout.write(`ok: lock checksums valid for ${selected.length} entries\n`);
  process.exit(0);
}

if (updated) {
  writeLock(resolvedLockPath, lock);
  process.stdout.write(`updated lock file: ${resolvedLockPath}\n`);
} else {
  process.stdout.write(`no lock changes needed: ${resolvedLockPath}\n`);
}
