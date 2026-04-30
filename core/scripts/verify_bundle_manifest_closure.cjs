#!/usr/bin/env node
const crypto = require("node:crypto");
const fs = require("node:fs");
const path = require("node:path");

function fail(message) {
  throw new Error(message);
}

function readJson(filePath, label) {
  try {
    return JSON.parse(fs.readFileSync(filePath, "utf8"));
  } catch (err) {
    fail(`failed to read ${label} at ${filePath}: ${err?.message ?? err}`);
  }
}

function resolveInside(rootDir, rawPath, label) {
  const value = String(rawPath || "").trim();
  if (!value) {
    fail(`${label} is required`);
  }
  if (path.isAbsolute(value)) {
    fail(`${label} must be bundle-relative: ${value}`);
  }
  const resolvedRoot = path.resolve(rootDir);
  const resolved = path.resolve(resolvedRoot, value);
  if (resolved !== resolvedRoot && !resolved.startsWith(`${resolvedRoot}${path.sep}`)) {
    fail(`${label} escapes the bundle directory: ${value}`);
  }
  return resolved;
}

function validateSha256(filePath, expected, label) {
  const digest = crypto.createHash("sha256").update(fs.readFileSync(filePath)).digest("hex");
  if (digest !== expected) {
    fail(`${label} sha256 mismatch: expected ${expected}, got ${digest}`);
  }
}

function validateFile(filePath, label, options = {}) {
  if (!fs.existsSync(filePath)) {
    fail(`${label} is missing: ${filePath}`);
  }
  const stat = fs.statSync(filePath);
  if (!stat.isFile()) {
    fail(`${label} is not a file: ${filePath}`);
  }
  if (options.executable) {
    fs.accessSync(filePath, fs.constants.X_OK);
  }
  const sha256 = String(options.sha256 || "").trim().toLowerCase();
  if (sha256) {
    if (!/^[a-f0-9]{64}$/.test(sha256)) {
      fail(`${label} has invalid sha256 metadata: ${sha256}`);
    }
    validateSha256(filePath, sha256, label);
  }
}

function validateProvider(bundleDir, provider, index) {
  const label = `manifest.providers[${index}]`;
  const command = resolveInside(bundleDir, provider?.command, `${label}.command`);
  validateFile(command, `${label}.command`, {
    executable: true,
    sha256: provider?.sha256,
  });
}

function validateRuntime(bundleDir, runtime, index) {
  const label = `manifest.runtimes[${index}]`;
  const root = resolveInside(bundleDir, runtime?.root, `${label}.root`);
  if (!fs.existsSync(root) || !fs.statSync(root).isDirectory()) {
    fail(`${label}.root is missing or not a directory: ${root}`);
  }
  const bin = resolveInside(root, runtime?.bin, `${label}.bin`);
  validateFile(bin, `${label}.bin`, {
    executable: true,
    sha256: runtime?.sha256,
  });
  const npmCli = String(runtime?.npm_cli || "").trim();
  if (npmCli) {
    const npmCliPath = resolveInside(root, npmCli, `${label}.npm_cli`);
    validateFile(npmCliPath, `${label}.npm_cli`);
  }
}

function validateImage(bundleDir, image, index) {
  const label = `manifest.images[${index}]`;
  const tar = resolveInside(bundleDir, image?.tar, `${label}.tar`);
  validateFile(tar, `${label}.tar`, {
    sha256: image?.sha256,
  });
}

function validateDaemon(bundleDir, daemon, index) {
  const label = `manifest.daemons[${index}]`;
  const bin = resolveInside(bundleDir, daemon?.bin, `${label}.bin`);
  validateFile(bin, `${label}.bin`, {
    executable: true,
    sha256: daemon?.sha256,
  });
}

function validateBundleManifestClosure(bundleDir) {
  const resolvedBundleDir = path.resolve(bundleDir);
  const manifestPath = path.join(resolvedBundleDir, "manifest.json");
  const manifest = readJson(manifestPath, "bundle manifest");
  if (manifest.version !== 1) {
    fail(`bundle manifest version must be 1, got ${manifest.version}`);
  }
  const providers = Array.isArray(manifest.providers) ? manifest.providers : [];
  const runtimes = Array.isArray(manifest.runtimes) ? manifest.runtimes : [];
  const images = Array.isArray(manifest.images) ? manifest.images : [];
  const daemons = Array.isArray(manifest.daemons) ? manifest.daemons : [];
  providers.forEach((provider, index) => validateProvider(resolvedBundleDir, provider, index));
  runtimes.forEach((runtime, index) => validateRuntime(resolvedBundleDir, runtime, index));
  images.forEach((image, index) => validateImage(resolvedBundleDir, image, index));
  daemons.forEach((daemon, index) => validateDaemon(resolvedBundleDir, daemon, index));
  return {
    providers: providers.length,
    runtimes: runtimes.length,
    images: images.length,
    daemons: daemons.length,
  };
}

function main() {
  const bundleDir = process.argv[2];
  if (!bundleDir) {
    console.error("usage: verify_bundle_manifest_closure.cjs <bundle-dir>");
    process.exit(2);
  }
  try {
    const result = validateBundleManifestClosure(bundleDir);
    console.log(
      `ok: bundle manifest closure verified providers=${result.providers} runtimes=${result.runtimes} images=${result.images} daemons=${result.daemons}`,
    );
  } catch (err) {
    console.error(`error: ${err?.message ?? err}`);
    process.exit(1);
  }
}

if (require.main === module) {
  main();
}

module.exports = {
  validateBundleManifestClosure,
};
