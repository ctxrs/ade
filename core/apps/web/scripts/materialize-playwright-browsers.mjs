#!/usr/bin/env node

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);

const usage = () => [
  "usage: node apps/web/scripts/materialize-playwright-browsers.mjs \\",
  "  --out-dir <path> \\",
  "  --runtime-manifest <path> \\",
  "  --browser <webkit|chromium|firefox> [--browser <...>]",
].join("\n");

const supportedBrowsers = new Set(["chromium", "firefox", "webkit"]);
const runfilesManifestCache = new Map();

const hasBazelRunfilesEnv = (env = process.env) => [
  env.RUNFILES_DIR,
  env.RUNFILES_MANIFEST_FILE,
  env.TEST_SRCDIR,
  env.TEST_WORKSPACE,
].some((value) => String(value || "").trim().length > 0);

const resolveRunfilesManifestPath = (env = process.env) => {
  const manifestPath = String(env.RUNFILES_MANIFEST_FILE || "").trim();
  if (!manifestPath || !fs.existsSync(manifestPath)) {
    return "";
  }
  return manifestPath;
};

const loadRunfilesManifest = (manifestPath) => {
  const cached = runfilesManifestCache.get(manifestPath);
  if (cached) {
    return cached;
  }
  const manifest = new Map();
  for (const line of fs.readFileSync(manifestPath, "utf8").split(/\r?\n/u)) {
    if (!line) {
      continue;
    }
    const separatorIndex = line.indexOf(" ");
    if (separatorIndex <= 0) {
      continue;
    }
    const logicalPath = line.slice(0, separatorIndex);
    const physicalPath = line.slice(separatorIndex + 1);
    if (!logicalPath || !physicalPath || logicalPath === physicalPath) {
      continue;
    }
    manifest.set(logicalPath, physicalPath);
  }
  runfilesManifestCache.set(manifestPath, manifest);
  return manifest;
};

const resolveManifestRoot = (manifestPath) => {
  if (manifestPath.endsWith("/MANIFEST")) {
    return manifestPath.slice(0, -"/MANIFEST".length);
  }
  if (manifestPath.endsWith("_manifest")) {
    return manifestPath.slice(0, -"_manifest".length);
  }
  return "";
};

const toRunfilesLogicalPath = (requestedPath) =>
  String(requestedPath || "").trim().replace(/\\/gu, "/").replace(/^\/+/u, "");

const resolveManifestCandidates = (requestedPath, env = process.env) => {
  const logicalPath = toRunfilesLogicalPath(requestedPath);
  if (!logicalPath) {
    return [];
  }
  const candidates = [logicalPath, `_main/${logicalPath}`];
  const workspaceName = String(env.TEST_WORKSPACE || "").trim();
  if (workspaceName) {
    candidates.push(`${workspaceName}/${logicalPath}`);
  }
  return candidates;
};

const resolveRunfilesPath = (requestedPath, env = process.env) => {
  const logicalPath = toRunfilesLogicalPath(requestedPath);
  if (!logicalPath) {
    return "";
  }
  for (const runfilesRoot of [env.RUNFILES_DIR, env.TEST_SRCDIR]) {
    const root = String(runfilesRoot || "").trim();
    if (!root) {
      continue;
    }
    for (const candidate of resolveManifestCandidates(logicalPath, env)) {
      const resolved = path.join(root, candidate);
      if (fs.existsSync(resolved)) {
        return resolved;
      }
    }
  }
  const manifestPath = resolveRunfilesManifestPath(env);
  if (!manifestPath) {
    return "";
  }
  const manifest = loadRunfilesManifest(manifestPath);
  for (const candidate of resolveManifestCandidates(logicalPath, env)) {
    const hit = manifest.get(candidate);
    if (hit && fs.existsSync(hit)) {
      return hit;
    }
  }
  const manifestRoot = resolveManifestRoot(manifestPath);
  if (!manifestRoot) {
    return "";
  }
  for (const candidate of resolveManifestCandidates(logicalPath, env)) {
    const resolved = path.join(manifestRoot, candidate);
    if (fs.existsSync(resolved)) {
      return resolved;
    }
  }
  return "";
};

const resolveByAscendingCwd = (configured) => {
  let current = process.cwd();
  while (true) {
    const candidate = path.resolve(current, configured);
    if (fs.existsSync(candidate)) {
      return candidate;
    }
    const parent = path.dirname(current);
    if (parent === current) {
      break;
    }
    current = parent;
  }
  return "";
};

const canWalkExecrootRelativePath = (raw, cwd = process.cwd()) =>
  raw.startsWith(".")
  || raw.startsWith("+")
  || (raw.startsWith("external/") && cwd.split(path.sep).includes("_main"));

const resolveExistingPath = (configured, env = process.env) => {
  const raw = String(configured || "").trim();
  if (!raw) {
    throw new Error("missing path");
  }
  const cwd = process.cwd();
  if (path.isAbsolute(raw)) {
    if (!fs.existsSync(raw)) {
      throw new Error(`declared runtime input does not exist: ${raw}`);
    }
    return raw;
  }
  const directRelativeCandidate = path.resolve(cwd, raw);
  if (fs.existsSync(directRelativeCandidate)) {
    return directRelativeCandidate;
  }
  const allowSandboxRelativeWalk = canWalkExecrootRelativePath(raw, cwd);
  if (allowSandboxRelativeWalk) {
    const walkedCandidate = resolveByAscendingCwd(raw);
    if (walkedCandidate) {
      return walkedCandidate;
    }
  }
  if (hasBazelRunfilesEnv(env)) {
    const runfilesResolved = resolveRunfilesPath(raw, env);
    if (runfilesResolved) {
      return runfilesResolved;
    }
    throw new Error(`declared runtime input does not exist: ${raw}`);
  }
  const walkedCandidate = resolveByAscendingCwd(raw);
  if (walkedCandidate) {
    return walkedCandidate;
  }
  throw new Error(`declared runtime input does not exist: ${raw}`);
};

export const parseArgs = (argv) => {
  const parsed = {
    browsers: [],
    outDir: "",
    runtimeManifest: "",
  };
  const args = [...argv];
  while (args.length > 0) {
    const arg = args.shift();
    switch (arg) {
      case "--out-dir":
        parsed.outDir = String(args.shift() || "").trim();
        break;
      case "--runtime-manifest":
        parsed.runtimeManifest = String(args.shift() || "").trim();
        break;
      case "--browser": {
        const browser = String(args.shift() || "").trim().toLowerCase();
        if (!supportedBrowsers.has(browser)) {
          throw new Error(`unsupported Playwright browser '${browser}'\n${usage()}`);
        }
        if (!parsed.browsers.includes(browser)) {
          parsed.browsers.push(browser);
        }
        break;
      }
      default:
        throw new Error(`unsupported argument: ${arg}\n${usage()}`);
    }
  }
  if (!parsed.outDir) {
    throw new Error(`missing --out-dir\n${usage()}`);
  }
  if (!parsed.runtimeManifest) {
    throw new Error(`missing --runtime-manifest\n${usage()}`);
  }
  if (parsed.browsers.length === 0) {
    throw new Error(`missing --browser\n${usage()}`);
  }
  return parsed;
};

const copyNodePreservingMetadata = (sourcePath, targetPath, ancestorRealDirs = new Set()) => {
  const sourceLStat = fs.lstatSync(sourcePath);
  let actualSourcePath = sourcePath;
  let sourceStats = sourceLStat;
  fs.mkdirSync(path.dirname(targetPath), { recursive: true });
  if (sourceLStat.isSymbolicLink()) {
    // EXCEPTION: Bazel tree artifacts do not support symlinks, so materialization
    // follows them and copies the target bytes instead of emitting output symlinks.
    actualSourcePath = fs.realpathSync.native(sourcePath);
    sourceStats = fs.statSync(actualSourcePath);
    if (sourceStats.isDirectory() && ancestorRealDirs.has(actualSourcePath)) {
      return;
    }
  }
  if (sourceStats.isDirectory()) {
    fs.mkdirSync(targetPath, { recursive: true });
    const nextAncestorRealDirs = new Set(ancestorRealDirs);
    nextAncestorRealDirs.add(fs.realpathSync.native(actualSourcePath));
    for (const entryName of fs.readdirSync(actualSourcePath).sort((left, right) => left.localeCompare(right))) {
      copyNodePreservingMetadata(
        path.join(actualSourcePath, entryName),
        path.join(targetPath, entryName),
        nextAncestorRealDirs,
      );
    }
    fs.chmodSync(targetPath, sourceStats.mode & 0o777);
    return;
  }
  if (!sourceStats.isFile()) {
    throw new Error(`unsupported Playwright runtime entry: ${sourcePath}`);
  }
  fs.copyFileSync(sourcePath, targetPath);
  fs.chmodSync(targetPath, sourceStats.mode & 0o777);
};

export const copyRuntimeTree = (sourceDir, targetDir) => {
  const sourceStats = fs.lstatSync(sourceDir);
  if (!sourceStats.isDirectory()) {
    throw new Error(`Playwright runtime tree is not a directory: ${sourceDir}`);
  }
  copyNodePreservingMetadata(sourceDir, targetDir);
};

export const materializePlaywrightBrowsers = (
  {
    browsers,
    outDir,
    runtimeManifest,
  },
  {
    env = process.env,
    copyRuntimeTreeImpl = copyRuntimeTree,
  } = {},
) => {
  const resolvedOutDir = path.resolve(process.cwd(), outDir);
  const resolvedManifestPath = resolveExistingPath(runtimeManifest, env);
  const runtimeManifestDir = path.dirname(resolvedManifestPath);
  const parsedManifest = JSON.parse(fs.readFileSync(resolvedManifestPath, "utf8"));
  const runtimePlatforms = parsedManifest?.platforms;
  if (!runtimePlatforms || typeof runtimePlatforms !== "object") {
    throw new Error(`invalid Playwright runtime manifest: ${resolvedManifestPath}`);
  }
  fs.rmSync(resolvedOutDir, { recursive: true, force: true });
  fs.mkdirSync(resolvedOutDir, { recursive: true });
  for (const [hostPlatform, runtimeBrowsers] of Object.entries(runtimePlatforms).sort(([left], [right]) =>
    left.localeCompare(right),
  )) {
    if (!runtimeBrowsers || typeof runtimeBrowsers !== "object") {
      throw new Error(`invalid Playwright runtime platform entry: ${hostPlatform}`);
    }
    for (const browser of browsers) {
      const entry = runtimeBrowsers[browser];
      if (!entry || typeof entry.directory !== "string" || !entry.directory.trim()) {
        throw new Error(`missing Playwright runtime entry for ${browser} on ${hostPlatform}`);
      }
      if (typeof entry.path !== "string" || !entry.path.trim()) {
        throw new Error(`missing Playwright runtime path for ${browser} on ${hostPlatform}`);
      }
      const runtimeDirName = entry.directory.trim();
      const runtimeTreePath = path.resolve(runtimeManifestDir, entry.path.trim());
      if (!fs.existsSync(runtimeTreePath) || !fs.statSync(runtimeTreePath).isDirectory()) {
        throw new Error(`missing Playwright runtime tree for ${browser} on ${hostPlatform}: ${runtimeTreePath}`);
      }
      const targetDir = path.join(resolvedOutDir, hostPlatform, runtimeDirName);
      copyRuntimeTreeImpl(runtimeTreePath, targetDir);
      fs.writeFileSync(path.join(targetDir, "INSTALLATION_COMPLETE"), `${browser}\n`);
    }
  }
  return resolvedOutDir;
};

const main = () => {
  const options = parseArgs(process.argv.slice(2));
  materializePlaywrightBrowsers(options);
};

if (process.argv[1] && path.resolve(process.argv[1]) === __filename) {
  try {
    main();
  } catch (error) {
    console.error(error instanceof Error ? error.message : String(error));
    process.exit(1);
  }
}
