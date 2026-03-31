#!/usr/bin/env node

const crypto = require("node:crypto");
const fs = require("node:fs");
const path = require("node:path");

const coreRoot = path.resolve(__dirname, "..");
const defaultLockPath = path.join(coreRoot, "apps", "desktop", "src-tauri", "bundles", "runtime_lock.v2.json");

const IMAGE_ID = "ctx-harness";
const IMAGE_VERSION_SEGMENT = "ubuntu-24.04";
const TARGET_OS = "linux";

const isNonEmptyString = (value) => typeof value === "string" && value.trim().length > 0;

const usage = () => {
  process.stdout.write(
    [
      "Usage:",
      "  node core/scripts/ctx_harness_lock_freshness.cjs --image-tar <docker-archive.tar> --arch <aarch64|x86_64>",
      "    [--lock-path <runtime_lock.v2.json>]",
      "",
      "Checks that the current ctx-harness docker-archive tar matches the managed",
      "runtime_lock.v2.json entry for the requested Linux arch.",
      "",
    ].join("\n"),
  );
};

const parseArgs = (argv) => {
  const cli = {
    imageTar: "",
    arch: "",
    lockPath: defaultLockPath,
  };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--image-tar") {
      cli.imageTar = String(argv[++i] || "").trim();
      continue;
    }
    if (arg === "--arch") {
      cli.arch = String(argv[++i] || "").trim();
      continue;
    }
    if (arg === "--lock-path") {
      cli.lockPath = String(argv[++i] || "").trim();
      continue;
    }
    if (arg === "-h" || arg === "--help") {
      usage();
      process.exit(0);
    }
    throw new Error(`unknown argument: ${arg}`);
  }
  if (!isNonEmptyString(cli.imageTar)) {
    throw new Error("--image-tar is required");
  }
  if (!isNonEmptyString(cli.arch)) {
    throw new Error("--arch is required");
  }
  if (!["aarch64", "x86_64"].includes(cli.arch)) {
    throw new Error(`unsupported --arch: ${cli.arch}`);
  }
  return cli;
};

const readJsonFile = (filePath) => JSON.parse(fs.readFileSync(filePath, "utf8"));
const sha256File = (filePath) => crypto.createHash("sha256").update(fs.readFileSync(filePath)).digest("hex");

const findImageComponent = (lock, arch) =>
  Array.isArray(lock?.components)
    ? lock.components.find(
      (component) => component
        && component.kind === "image"
        && component.id === IMAGE_ID
        && component.os === TARGET_OS
        && component.arch === arch
        && String(component.variant || "default").trim() === "default",
    ) || null
    : null;

const expectedObjectPath = (arch, sha256) => path.posix.join(
  "images",
  IMAGE_ID,
  IMAGE_VERSION_SEGMENT,
  TARGET_OS,
  arch,
  `sha256-${sha256}`,
  `ctx-harness-linux-${arch}.tar`,
);

const expectedUriSuffix = (arch, sha256) => `/${expectedObjectPath(arch, sha256)}`;

const validateCtxHarnessLockFreshness = ({ imageTar, lock, arch }) => {
  const errors = [];
  if (!fs.existsSync(imageTar)) {
    errors.push(`image tar is missing: ${imageTar}`);
    return { ok: false, errors };
  }

  const component = findImageComponent(lock, arch);
  if (!component) {
    errors.push(`runtime_lock.v2.json is missing image component ${IMAGE_ID} for ${TARGET_OS}/${arch}`);
    return { ok: false, errors };
  }

  const actualSha = sha256File(imageTar);
  const localSource = Array.isArray(component.sources)
    ? component.sources.find((source) => source?.source_type === "local") || null
    : null;
  const ciSource = Array.isArray(component.sources)
    ? component.sources.find((source) => source?.source_type === "ci") || null
    : null;

  if (!localSource || String(localSource.build || "").trim() !== "desktop:prep") {
    errors.push(`runtime lock is missing the local desktop:prep source for ${IMAGE_ID} ${TARGET_OS}/${arch}`);
  }
  if (!ciSource) {
    errors.push(`runtime lock is missing the ci source for ${IMAGE_ID} ${TARGET_OS}/${arch}`);
  } else {
    if (String(ciSource.sha256 || "").trim() !== actualSha) {
      errors.push(`runtime lock ci sha drift: expected ${actualSha}, found ${String(ciSource.sha256 || "")}`);
    }
    const expectedSuffix = expectedUriSuffix(arch, actualSha);
    if (!String(ciSource.uri || "").includes(expectedSuffix)) {
      errors.push(`runtime lock ci uri drift: expected suffix ${expectedSuffix}, found ${String(ciSource.uri || "")}`);
    }
  }

  return {
    ok: errors.length === 0,
    errors,
    sha256: actualSha,
    expectedObjectPath: expectedObjectPath(arch, actualSha),
    expectedUriSuffix: expectedUriSuffix(arch, actualSha),
  };
};

const main = () => {
  const cli = parseArgs(process.argv.slice(2));
  const lock = readJsonFile(path.resolve(cli.lockPath));
  const result = validateCtxHarnessLockFreshness({
    imageTar: path.resolve(cli.imageTar),
    lock,
    arch: cli.arch,
  });

  if (!result.ok) {
    for (const error of result.errors) {
      process.stderr.write(`error: ${error}\n`);
    }
    process.exit(1);
  }

  process.stdout.write(
    `ctx_harness_lock_freshness: OK (${cli.arch}) sha256=${result.sha256} object=${result.expectedObjectPath}\n`,
  );
};

if (require.main === module) {
  try {
    main();
  } catch (error) {
    process.stderr.write(`error: ${error instanceof Error ? error.message : String(error)}\n`);
    process.exit(1);
  }
}

module.exports = {
  expectedObjectPath,
  expectedUriSuffix,
  findImageComponent,
  validateCtxHarnessLockFreshness,
};
