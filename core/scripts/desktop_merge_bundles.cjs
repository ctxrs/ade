#!/usr/bin/env node
const crypto = require("crypto");
const fs = require("fs");
const path = require("path");
const util = require("util");

const usage = () => {
  console.error(
    [
      "usage:",
      "  node core/scripts/desktop_merge_bundles.cjs \\",
      "    --input <bundle-dir> [--input <bundle-dir> ...] --output <bundle-dir> \\",
      "    [--require-provider id:os:arch] [--require-runtime id:os:arch] [--require-image id:os:arch] [--require-daemon id:os:arch]",
    ].join("\n"),
  );
};

const parseArgs = (argv) => {
  const args = {
    inputs: [],
    output: "",
    requireProvider: [],
    requireRuntime: [],
    requireImage: [],
    requireDaemon: [],
  };
  for (let i = 0; i < argv.length; i += 1) {
    const flag = argv[i];
    const value = argv[i + 1];
    switch (flag) {
      case "--input":
        if (!value) throw new Error("missing value for --input");
        args.inputs.push(value);
        i += 1;
        break;
      case "--output":
        if (!value) throw new Error("missing value for --output");
        args.output = value;
        i += 1;
        break;
      case "--require-provider":
        if (!value) throw new Error("missing value for --require-provider");
        args.requireProvider.push(value);
        i += 1;
        break;
      case "--require-runtime":
        if (!value) throw new Error("missing value for --require-runtime");
        args.requireRuntime.push(value);
        i += 1;
        break;
      case "--require-image":
        if (!value) throw new Error("missing value for --require-image");
        args.requireImage.push(value);
        i += 1;
        break;
      case "--require-daemon":
        if (!value) throw new Error("missing value for --require-daemon");
        args.requireDaemon.push(value);
        i += 1;
        break;
      default:
        throw new Error(`unknown argument: ${flag}`);
    }
  }
  if (args.inputs.length < 1) throw new Error("expected at least one --input value");
  if (!args.output) throw new Error("missing required --output");
  return args;
};

const normalizeBundleDir = (dir) => {
  const abs = path.resolve(dir);
  const manifestPath = path.join(abs, "manifest.json");
  if (!fs.existsSync(manifestPath)) {
    throw new Error(`bundle dir missing manifest.json: ${abs}`);
  }
  return abs;
};

const parseManifest = (bundleDir) => {
  const manifestPath = path.join(bundleDir, "manifest.json");
  let parsed;
  try {
    parsed = JSON.parse(fs.readFileSync(manifestPath, "utf8"));
  } catch (err) {
    throw new Error(`failed to parse manifest: ${manifestPath}: ${err?.message ?? err}`);
  }
  if (parsed?.version !== 1) {
    throw new Error(`unsupported manifest version in ${manifestPath}: ${String(parsed?.version)}`);
  }
  return parsed;
};

const entryKey = (entry) => `${entry.id}::${entry.os}::${entry.arch}`;

const mergeSection = (sectionName, manifests) => {
  const merged = new Map();
  for (const manifest of manifests) {
    const entries = Array.isArray(manifest[sectionName]) ? manifest[sectionName] : [];
    for (const entry of entries) {
      if (!entry || typeof entry !== "object") {
        throw new Error(`invalid ${sectionName} entry: ${util.inspect(entry)}`);
      }
      if (!entry.id || !entry.os || !entry.arch) {
        throw new Error(`missing id/os/arch in ${sectionName} entry: ${JSON.stringify(entry)}`);
      }
      const key = entryKey(entry);
      const prev = merged.get(key);
      if (prev) {
        if (!util.isDeepStrictEqual(prev, entry)) {
          throw new Error(
            `conflicting ${sectionName} entry for ${key}\nprev=${JSON.stringify(
              prev,
            )}\nnext=${JSON.stringify(entry)}`,
          );
        }
        continue;
      }
      merged.set(key, entry);
    }
  }
  return [...merged.entries()]
    .sort((a, b) => a[0].localeCompare(b[0]))
    .map(([, value]) => value);
};

const hashFile = (filePath) => {
  const data = fs.readFileSync(filePath);
  return crypto.createHash("sha256").update(data).digest("hex");
};

const ensureDir = (dir) => {
  fs.mkdirSync(dir, { recursive: true });
};

const copyTreeWithConflictCheck = (sourceDir, outputDir) => {
  const stack = [""];
  while (stack.length > 0) {
    const rel = stack.pop();
    const fromPath = path.join(sourceDir, rel);
    for (const entry of fs.readdirSync(fromPath, { withFileTypes: true })) {
      const entryRel = rel ? path.join(rel, entry.name) : entry.name;
      if (entryRel === "manifest.json") continue;
      const srcPath = path.join(sourceDir, entryRel);
      const dstPath = path.join(outputDir, entryRel);
      if (entry.isDirectory()) {
        ensureDir(dstPath);
        stack.push(entryRel);
        continue;
      }
      if (!entry.isFile()) continue;
      ensureDir(path.dirname(dstPath));
      if (!fs.existsSync(dstPath)) {
        fs.copyFileSync(srcPath, dstPath);
        continue;
      }
      const srcSha = hashFile(srcPath);
      const dstSha = hashFile(dstPath);
      if (srcSha !== dstSha) {
        throw new Error(`file content conflict at ${entryRel}: ${srcSha} vs ${dstSha}`);
      }
    }
  }
};

const parseRequirement = (raw) => {
  const parts = raw.split(":");
  if (parts.length !== 3) {
    throw new Error(`invalid requirement '${raw}', expected id:os:arch`);
  }
  return { id: parts[0], os: parts[1], arch: parts[2] };
};

const requireEntries = (entries, rawRequirements, sectionName) => {
  const requirements = rawRequirements.map(parseRequirement);
  for (const req of requirements) {
    const found = entries.some((entry) => entry.id === req.id && entry.os === req.os && entry.arch === req.arch);
    if (!found) {
      throw new Error(`missing required ${sectionName} entry: ${req.id}:${req.os}:${req.arch}`);
    }
  }
};

const main = () => {
  let options;
  try {
    options = parseArgs(process.argv.slice(2));
  } catch (err) {
    usage();
    throw err;
  }

  const inputDirs = options.inputs.map(normalizeBundleDir);
  const outputDir = path.resolve(options.output);

  fs.rmSync(outputDir, { recursive: true, force: true });
  ensureDir(outputDir);

  const manifests = inputDirs.map(parseManifest);
  for (const dir of inputDirs) {
    copyTreeWithConflictCheck(dir, outputDir);
  }

  const providers = mergeSection("providers", manifests);
  const runtimes = mergeSection("runtimes", manifests);
  const images = mergeSection("images", manifests);
  const daemons = mergeSection("daemons", manifests);

  requireEntries(providers, options.requireProvider, "provider");
  requireEntries(runtimes, options.requireRuntime, "runtime");
  requireEntries(images, options.requireImage, "image");
  requireEntries(daemons, options.requireDaemon, "daemon");

  const mergedManifest = {
    version: 1,
    generated_at: new Date().toISOString(),
    providers,
    runtimes,
    images,
    daemons,
  };
  fs.writeFileSync(path.join(outputDir, "manifest.json"), `${JSON.stringify(mergedManifest, null, 2)}\n`, "utf8");
};

main();
