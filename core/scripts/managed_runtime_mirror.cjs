#!/usr/bin/env node
"use strict";

const crypto = require("node:crypto");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { Readable } = require("node:stream");
const { pipeline } = require("node:stream/promises");

const CORE_ROOT = path.resolve(__dirname, "..");
const DEFAULT_LOCK_PATH = path.join(CORE_ROOT, "crates", "ctx-managed-installs", "src", "runtime_lock.rs");
const MIRROR_HOST = "api.ctx.rs";
const MIRROR_PUBLIC_PREFIX = "/storage/v1/object/public/";
const MANAGED_RUNTIME_OBJECT_PREFIX = "artifacts/managed-runtimes/";
const DEFAULT_BUCKET = "releases";

function usage() {
  return `Usage: node scripts/managed_runtime_mirror.cjs [mode] [options]

Modes:
  --list-json         Print parsed runtime lock entries and exit.
  --check-presence   Check every mirror URL returns a successful response.
  --verify           Download every mirror object and verify SHA-256.
  --publish          Download upstream artifacts, verify SHA-256, and upload immutable mirror objects.

Options:
  --lock <path>          Runtime lock source file (default: ${path.relative(CORE_ROOT, DEFAULT_LOCK_PATH)})
  --cache-dir <path>     Artifact cache directory (default: $CTX_VOLATILE_TMPDIR/managed-runtime-mirror)
  --dry-run             With --publish, print planned uploads without downloading or uploading.
  --repair-mismatched   With --publish, replace existing mirror objects that do not match the lock SHA.
  --verify-after-publish Verify mirror SHA-256 after publish.

Publish env:
  SUPABASE_URL
  SUPABASE_SERVICE_ROLE_KEY
  SUPABASE_STORAGE_BUCKET (optional, defaults to ${DEFAULT_BUCKET}; must match lock bucket)
`;
}

function parseArgs(argv) {
  const options = {
    mode: "",
    lockPath: DEFAULT_LOCK_PATH,
    cacheDir: path.join(process.env.CTX_VOLATILE_TMPDIR || os.tmpdir(), "managed-runtime-mirror"),
    dryRun: false,
    repairMismatched: false,
    verifyAfterPublish: false,
  };
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "--list-json") {
      options.mode = "list-json";
    } else if (arg === "--check-presence") {
      options.mode = "check-presence";
    } else if (arg === "--verify") {
      options.mode = "verify";
    } else if (arg === "--publish") {
      options.mode = "publish";
    } else if (arg === "--lock") {
      index += 1;
      options.lockPath = argv[index] || "";
    } else if (arg === "--cache-dir") {
      index += 1;
      options.cacheDir = argv[index] || "";
    } else if (arg === "--dry-run") {
      options.dryRun = true;
    } else if (arg === "--repair-mismatched") {
      options.repairMismatched = true;
    } else if (arg === "--verify-after-publish") {
      options.verifyAfterPublish = true;
    } else if (arg === "--help" || arg === "-h") {
      options.mode = "help";
    } else {
      throw new Error(`unsupported argument: ${arg}`);
    }
  }
  if (!options.mode) {
    options.mode = "check-presence";
  }
  if (!options.lockPath) {
    throw new Error("--lock requires a path");
  }
  if (!options.cacheDir) {
    throw new Error("--cache-dir requires a path");
  }
  return options;
}

function parseStringField(block, fieldName) {
  const match = block.match(new RegExp(`${fieldName}:\\s*"([^"]+)"`));
  if (!match) {
    throw new Error(`runtime lock entry is missing ${fieldName}`);
  }
  return match[1];
}

function parseBuildTag(block) {
  const someMatch = block.match(/build_tag:\s*Some\("([^"]+)"\)/);
  if (someMatch) {
    return someMatch[1];
  }
  if (/build_tag:\s*None/.test(block)) {
    return null;
  }
  throw new Error("runtime lock entry is missing build_tag");
}

function parseRuntimeLockText(text) {
  const entries = [];
  const entryPattern = /ManagedRuntimeArchiveSpec\s*\{([\s\S]*?)\n\s*\}/g;
  for (const match of text.matchAll(entryPattern)) {
    const block = match[1];
    if (!/archive_name:/.test(block) || !/mirror_url:/.test(block)) {
      continue;
    }
    const kindMatch = block.match(/kind:\s*ManagedRuntimeKind::([A-Za-z]+)/);
    if (!kindMatch) {
      if (/pub\(crate\)/.test(block)) {
        continue;
      }
      throw new Error("runtime lock entry is missing kind");
    }
    const kind = kindMatch[1].toLowerCase();
    const archiveName = parseStringField(block, "archive_name");
    const mirrorUrl = parseStringField(block, "mirror_url");
    const sha256 = parseStringField(block, "sha256").toLowerCase();
    if (!/^[0-9a-f]{64}$/.test(sha256)) {
      throw new Error(`invalid SHA-256 for ${archiveName}: ${sha256}`);
    }
    const mirror = resolveMirrorObject(mirrorUrl);
    const entry = {
      kind,
      version: parseStringField(block, "version"),
      buildTag: parseBuildTag(block),
      target: parseStringField(block, "target"),
      archiveName,
      mirrorUrl,
      sha256,
      bucket: mirror.bucket,
      objectPath: mirror.objectPath,
    };
    entry.upstreamUrl = buildUpstreamUrl(entry);
    entries.push(entry);
  }
  if (entries.length === 0) {
    throw new Error("runtime lock has no ManagedRuntimeArchiveSpec entries");
  }
  return entries;
}

function parseRuntimeLock(lockPath = DEFAULT_LOCK_PATH) {
  return parseRuntimeLockText(fs.readFileSync(lockPath, "utf8"));
}

function resolveMirrorObject(mirrorUrl) {
  const parsed = new URL(mirrorUrl);
  if (parsed.protocol !== "https:") {
    throw new Error(`managed runtime mirror URL must use https: ${mirrorUrl}`);
  }
  if (parsed.hostname !== MIRROR_HOST) {
    throw new Error(`managed runtime mirror URL must use ${MIRROR_HOST}: ${mirrorUrl}`);
  }
  if (!parsed.pathname.startsWith(MIRROR_PUBLIC_PREFIX)) {
    throw new Error(`managed runtime mirror URL must be a public Storage object: ${mirrorUrl}`);
  }
  const publicPath = parsed.pathname.slice(MIRROR_PUBLIC_PREFIX.length);
  const slash = publicPath.indexOf("/");
  if (slash <= 0) {
    throw new Error(`managed runtime mirror URL is missing bucket/object path: ${mirrorUrl}`);
  }
  const bucket = publicPath.slice(0, slash);
  const objectPath = publicPath.slice(slash + 1);
  if (bucket !== DEFAULT_BUCKET) {
    throw new Error(`managed runtime mirror URL must use ${DEFAULT_BUCKET} bucket: ${mirrorUrl}`);
  }
  if (!objectPath.startsWith(MANAGED_RUNTIME_OBJECT_PREFIX)) {
    throw new Error(`managed runtime mirror object must stay under ${MANAGED_RUNTIME_OBJECT_PREFIX}: ${mirrorUrl}`);
  }
  return { bucket, objectPath };
}

function buildUpstreamUrl(entry) {
  if (entry.kind === "node") {
    return `https://nodejs.org/dist/v${entry.version}/${entry.archiveName}`;
  }
  if (entry.kind === "python") {
    if (!entry.buildTag) {
      throw new Error(`Python runtime ${entry.archiveName} is missing build_tag`);
    }
    return `https://github.com/indygreg/python-build-standalone/releases/download/${entry.buildTag}/${entry.archiveName}`;
  }
  throw new Error(`unsupported managed runtime kind: ${entry.kind}`);
}

function contentTypeForArchive(archiveName) {
  if (archiveName.endsWith(".zip")) {
    return "application/zip";
  }
  if (archiveName.endsWith(".tar.gz") || archiveName.endsWith(".tgz")) {
    return "application/gzip";
  }
  return "application/octet-stream";
}

function sha256File(filePath) {
  return new Promise((resolve, reject) => {
    const hash = crypto.createHash("sha256");
    const input = fs.createReadStream(filePath);
    input.on("error", reject);
    input.on("data", (chunk) => hash.update(chunk));
    input.on("end", () => resolve(hash.digest("hex")));
  });
}

function assertAllowedFinalMirrorUrl(originalUrl, finalUrl) {
  const original = resolveMirrorObject(originalUrl);
  const final = resolveMirrorObject(finalUrl);
  if (original.bucket !== final.bucket || original.objectPath !== final.objectPath) {
    throw new Error(`managed runtime mirror redirected to a different object: ${originalUrl} -> ${finalUrl}`);
  }
}

async function fetchToFile(url, destination, { headers = {} } = {}) {
  const response = await fetch(url, { headers, redirect: "follow" });
  if (!response.ok) {
    const body = await response.text().catch(() => "");
    throw new Error(`GET ${url} failed with HTTP ${response.status}${body ? `: ${body.slice(0, 200)}` : ""}`);
  }
  if (url.includes(`://${MIRROR_HOST}/`)) {
    assertAllowedFinalMirrorUrl(url, response.url);
  }
  await fs.promises.mkdir(path.dirname(destination), { recursive: true });
  await pipeline(Readable.fromWeb(response.body), fs.createWriteStream(destination));
}

async function downloadAndVerify(url, destination, expectedSha256) {
  await fetchToFile(url, destination);
  const actualSha256 = await sha256File(destination);
  if (actualSha256 !== expectedSha256) {
    throw new Error(`SHA-256 mismatch for ${url}: expected ${expectedSha256}, got ${actualSha256}`);
  }
}

async function checkPresence(entry) {
  const response = await fetch(entry.mirrorUrl, {
    method: "GET",
    headers: { range: "bytes=0-0" },
    redirect: "follow",
  });
  if (!response.ok && response.status !== 206) {
    const body = await response.text().catch(() => "");
    throw new Error(`${entry.archiveName}: mirror returned HTTP ${response.status}${body ? `: ${body.slice(0, 200)}` : ""}`);
  }
  assertAllowedFinalMirrorUrl(entry.mirrorUrl, response.url);
  if (response.body) {
    await response.body.cancel().catch(() => {});
  }
}

function assertPublishEnv(env, entries) {
  const supabaseUrl = String(env.SUPABASE_URL || "").trim();
  const token = String(env.SUPABASE_SERVICE_ROLE_KEY || "");
  const bucket = String(env.SUPABASE_STORAGE_BUCKET || DEFAULT_BUCKET);
  if (!supabaseUrl || !token) {
    throw new Error("publishing requires SUPABASE_URL and SUPABASE_SERVICE_ROLE_KEY");
  }
  const parsedUrl = new URL(supabaseUrl);
  const rootPath = parsedUrl.pathname === "" || parsedUrl.pathname === "/";
  if (
    parsedUrl.protocol !== "https:"
    || parsedUrl.hostname !== MIRROR_HOST
    || parsedUrl.port !== ""
    || parsedUrl.username !== ""
    || parsedUrl.password !== ""
    || parsedUrl.search !== ""
    || parsedUrl.hash !== ""
    || !rootPath
  ) {
    throw new Error(`publishing requires SUPABASE_URL=https://${MIRROR_HOST}`);
  }
  for (const entry of entries) {
    if (entry.bucket !== bucket) {
      throw new Error(`SUPABASE_STORAGE_BUCKET=${bucket} does not match lock bucket ${entry.bucket}`);
    }
  }
  return { supabaseUrl: `https://${MIRROR_HOST}`, token, bucket };
}

async function verifyExistingObjectMatches(entry, localPath) {
  const actualPath = `${localPath}.mirror`;
  await downloadAndVerify(entry.mirrorUrl, actualPath, entry.sha256);
  await fs.promises.rm(actualPath, { force: true });
}

async function uploadObject({ entry, localPath, supabaseUrl, token, bucket }) {
  const sizeBytes = fs.statSync(localPath).size;
  const uploadUrl = `${supabaseUrl}/storage/v1/object/${bucket}/${entry.objectPath}`;
  const response = await fetch(uploadUrl, {
    method: "POST",
    headers: {
      authorization: `Bearer ${token}`,
      apikey: token,
      "content-type": contentTypeForArchive(entry.archiveName),
      "content-length": String(sizeBytes),
      "x-upsert": "false",
    },
    body: fs.createReadStream(localPath),
    duplex: "half",
  });
  if (response.ok) {
    return "uploaded";
  }
  const body = await response.text().catch(() => "");
  if (response.status === 400 || response.status === 409) {
    await verifyExistingObjectMatches(entry, localPath);
    return "already-present";
  }
  throw new Error(`upload failed for ${entry.objectPath}: HTTP ${response.status}${body ? `: ${body.slice(0, 300)}` : ""}`);
}

async function deleteObject({ entry, supabaseUrl, token, bucket }) {
  const deleteUrl = `${supabaseUrl}/storage/v1/object/${bucket}/${entry.objectPath}`;
  const response = await fetch(deleteUrl, {
    method: "DELETE",
    headers: {
      authorization: `Bearer ${token}`,
      apikey: token,
    },
  });
  if (!response.ok && response.status !== 404) {
    const body = await response.text().catch(() => "");
    throw new Error(`delete failed for ${entry.objectPath}: HTTP ${response.status}${body ? `: ${body.slice(0, 300)}` : ""}`);
  }
}

async function publishObject({ entry, localPath, publishEnv, repairMismatched }) {
  try {
    return await uploadObject({ entry, localPath, ...publishEnv });
  } catch (error) {
    if (!repairMismatched || !/SHA-256 mismatch/u.test(String(error.message || ""))) {
      throw error;
    }
    console.log(`repair-mismatched\t${entry.objectPath}`);
    await deleteObject({ entry, ...publishEnv });
    return await uploadObject({ entry, localPath, ...publishEnv });
  }
}

async function publishEntries(entries, options, env = process.env) {
  const publishEnv = options.dryRun ? null : assertPublishEnv(env, entries);
  await fs.promises.mkdir(options.cacheDir, { recursive: true });
  for (const entry of entries) {
    const localPath = path.join(options.cacheDir, entry.archiveName);
    if (options.dryRun) {
      console.log(`publish-plan\t${entry.kind}\t${entry.target}\t${entry.upstreamUrl}\t${entry.mirrorUrl}`);
      continue;
    }
    if (fs.existsSync(localPath) && await sha256File(localPath) === entry.sha256) {
      console.log(`cached\t${entry.archiveName}`);
    } else {
      console.log(`download\t${entry.archiveName}\t${entry.upstreamUrl}`);
      await downloadAndVerify(entry.upstreamUrl, localPath, entry.sha256);
    }
    const status = await publishObject({
      entry,
      localPath,
      publishEnv,
      repairMismatched: options.repairMismatched,
    });
    console.log(`${status}\t${entry.objectPath}`);
  }
  if (options.verifyAfterPublish && !options.dryRun) {
    await verifyEntries(entries, options);
  }
}

async function verifyEntries(entries, options) {
  await fs.promises.mkdir(options.cacheDir, { recursive: true });
  for (const entry of entries) {
    const localPath = path.join(options.cacheDir, "mirror", entry.archiveName);
    console.log(`verify\t${entry.archiveName}\t${entry.mirrorUrl}`);
    await downloadAndVerify(entry.mirrorUrl, localPath, entry.sha256);
  }
}

async function checkEntries(entries) {
  for (const entry of entries) {
    console.log(`check\t${entry.archiveName}\t${entry.mirrorUrl}`);
    await checkPresence(entry);
  }
}

async function main(argv = process.argv.slice(2), env = process.env) {
  const options = parseArgs(argv);
  if (options.mode === "help") {
    console.log(usage());
    return 0;
  }
  const entries = parseRuntimeLock(options.lockPath);
  if (options.mode === "list-json") {
    console.log(JSON.stringify({ entries }, null, 2));
    return 0;
  }
  if (options.mode === "check-presence") {
    await checkEntries(entries);
    return 0;
  }
  if (options.mode === "verify") {
    await verifyEntries(entries, options);
    return 0;
  }
  if (options.mode === "publish") {
    await publishEntries(entries, options, env);
    return 0;
  }
  throw new Error(`unsupported mode: ${options.mode}`);
}

if (require.main === module) {
  main().catch((error) => {
    console.error(`error: ${error.message}`);
    process.exit(1);
  });
}

module.exports = {
  DEFAULT_LOCK_PATH,
  assertPublishEnv,
  buildUpstreamUrl,
  contentTypeForArchive,
  parseArgs,
  parseRuntimeLock,
  parseRuntimeLockText,
  resolveMirrorObject,
};
