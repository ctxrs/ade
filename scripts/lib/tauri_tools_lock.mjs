#!/usr/bin/env node
import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";

export const ROOT_DIR = path.resolve(new URL("../../", import.meta.url).pathname);
export const DEFAULT_LOCK_PATH = path.join(
  ROOT_DIR,
  "core/apps/desktop/src-tauri/bundles/tauri_tools_lock.v1.json",
);

export const SUPPORTED_PLATFORMS = new Set(["linux-x64", "linux-arm64"]);
export const DEFAULT_SUPABASE_STORAGE_BUCKET = "releases";

export const normalizePlatform = (value) => {
  const platform = String(value || "").trim();
  if (!platform) return "";
  if (!SUPPORTED_PLATFORMS.has(platform)) {
    throw new Error(`unsupported platform: ${platform}`);
  }
  return platform;
};

export const loadLock = (lockPath) => {
  const resolved = path.resolve(lockPath || DEFAULT_LOCK_PATH);
  const raw = fs.readFileSync(resolved, "utf8");
  const parsed = JSON.parse(raw);
  if (!parsed || typeof parsed !== "object") {
    throw new Error(`invalid lock file: ${resolved}`);
  }
  if (parsed.schema_version !== 1) {
    throw new Error(`unsupported lock schema_version: ${String(parsed.schema_version ?? "")}`);
  }
  if (!Array.isArray(parsed.entries) || parsed.entries.length === 0) {
    throw new Error(`lock entries missing: ${resolved}`);
  }
  return { lockPath: resolved, lock: parsed };
};

export const writeLock = (lockPath, lock) => {
  const next = {
    ...lock,
    updated_at: new Date().toISOString(),
    entries: [...lock.entries].sort((a, b) => String(a.id).localeCompare(String(b.id))),
  };
  fs.writeFileSync(lockPath, `${JSON.stringify(next, null, 2)}\n`, "utf8");
};

export const selectEntries = (entries, platform) => {
  if (!platform) return entries;
  return entries.filter((entry) => Array.isArray(entry.platforms) && entry.platforms.includes(platform));
};

export const upstreamUrlFor = (entry) =>
  `https://github.com/${encodeURIComponent(entry.owner)}/${encodeURIComponent(entry.repo)}/releases/download/${encodeURIComponent(entry.tag)}/${encodeURIComponent(entry.asset)}`;

export const mirrorRelativePathFor = (entry) =>
  `tauri-tools/${entry.owner}/${entry.repo}/releases/download/${entry.tag}/${entry.asset}`;

export const mirrorUrlFor = (baseUrl, entry) => {
  const base = String(baseUrl || "").trim().replace(/\/+$/, "");
  if (!base) throw new Error("missing mirror base URL");
  return `${base}/${entry.owner}/${entry.repo}/releases/download/${entry.tag}/${entry.asset}`;
};

export const resolveSupabaseStorageBucket = () =>
  String(process.env.SUPABASE_STORAGE_BUCKET || "").trim() || DEFAULT_SUPABASE_STORAGE_BUCKET;

export const resolveMirrorBaseUrl = () => {
  const explicit = String(process.env.TAURI_TOOLS_MIRROR_BASE_URL || "").trim().replace(/\/+$/, "");
  if (explicit) return explicit;
  const supabaseUrl = String(process.env.SUPABASE_URL || "").trim().replace(/\/+$/, "");
  if (!supabaseUrl) return "";
  const bucket = resolveSupabaseStorageBucket();
  return `${supabaseUrl}/storage/v1/object/public/${bucket}/tauri-tools`;
};

export const lockEntryLabel = (entry) => `${entry.id} (${entry.asset})`;

export const sha256Hex = (buffer) => crypto.createHash("sha256").update(buffer).digest("hex");

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

export const fetchBufferWithRetry = async (url, attempts = 4) => {
  let lastError = null;
  for (let attempt = 1; attempt <= attempts; attempt += 1) {
    try {
      const response = await fetch(url, { redirect: "follow" });
      if (!response.ok) {
        throw new Error(`HTTP ${response.status} (${response.statusText})`);
      }
      const buffer = Buffer.from(await response.arrayBuffer());
      return buffer;
    } catch (error) {
      lastError = error;
      if (attempt < attempts) {
        await sleep(attempt * 1000);
      }
    }
  }
  throw new Error(`failed to download ${url}: ${String(lastError instanceof Error ? lastError.message : lastError)}`);
};

export const ensureEntrySchema = (entry) => {
  const required = ["id", "owner", "repo", "tag", "asset", "cache_filename", "sha256", "size_bytes", "platforms"];
  for (const key of required) {
    if (!(key in entry)) {
      throw new Error(`lock entry missing key '${key}': ${JSON.stringify(entry)}`);
    }
  }
  if (!Array.isArray(entry.platforms) || entry.platforms.length === 0) {
    throw new Error(`lock entry has empty platforms: ${entry.id}`);
  }
  for (const platform of entry.platforms) {
    normalizePlatform(platform);
  }
};
