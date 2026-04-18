const childProcess = require("node:child_process");
const crypto = require("node:crypto");
const fs = require("node:fs");
const path = require("node:path");

const {
  buildCtxCacheEnv,
  resolveConfiguredPath,
  resolveCtxCacheLayout,
} = require("./cache_roots.cjs");
const { HOST_HEAVY_BUDGET_KEY, withHostJobBudget } = require("./host_job_budget.cjs");
const { bazeliskBinaryPath, buildBuildBuddyAuthArgs } = require("../run_bazel_pilot.cjs");

const ARTIFACT_VERSION = 1;
const ARTIFACT_MARKER = ".ctx-web-dist-artifact.json";
const BAZEL_VERSION_FILE = ".bazelversion";
const WEB_DIST_SYNC_TARGET = "//core/apps/web:dist_sync";
const IGNORED_DIR_NAMES = new Set([
  ".git",
  ".turbo",
  "coverage",
  "dist",
  "node_modules",
  "playwright-report",
  "test-results",
]);
const DEFAULT_INPUTS = [
  "apps/web",
  "packages",
  "package.json",
  "pnpm-lock.yaml",
  "pnpm-workspace.yaml",
];

function trimValue(value) {
  return String(value ?? "").trim();
}

function updateHashWithFile(hash, absolutePath, relativePath, stats) {
  hash.update(`file:${relativePath}:${stats.mode}:${stats.size}\n`);
  hash.update(fs.readFileSync(absolutePath));
  hash.update("\n");
}

function updateHashWithSymlink(hash, absolutePath, relativePath) {
  hash.update(`symlink:${relativePath}:${fs.readlinkSync(absolutePath)}\n`);
}

function updateHashWithMissingPath(hash, relativePath) {
  hash.update(`missing:${relativePath}\n`);
}

function walkForFingerprint({ hash, rootDir, currentRelativePath }) {
  const absolutePath = path.join(rootDir, currentRelativePath);
  if (!fs.existsSync(absolutePath)) {
    updateHashWithMissingPath(hash, currentRelativePath);
    return;
  }

  const stats = fs.lstatSync(absolutePath);
  if (stats.isSymbolicLink()) {
    updateHashWithSymlink(hash, absolutePath, currentRelativePath);
    return;
  }
  if (stats.isFile()) {
    updateHashWithFile(hash, absolutePath, currentRelativePath, stats);
    return;
  }
  if (!stats.isDirectory()) {
    hash.update(`other:${currentRelativePath}:${stats.mode}\n`);
    return;
  }

  hash.update(`dir:${currentRelativePath}:${stats.mode}\n`);
  const entries = fs.readdirSync(absolutePath, { withFileTypes: true })
    .filter((entry) => !IGNORED_DIR_NAMES.has(entry.name))
    .sort((left, right) => left.name.localeCompare(right.name));
  for (const entry of entries) {
    const nextRelativePath = path.join(currentRelativePath, entry.name);
    walkForFingerprint({ hash, rootDir, currentRelativePath: nextRelativePath });
  }
}

function computeWebDistCacheKey({
  coreRoot,
  appVersion = "",
  extra = {},
  inputs = DEFAULT_INPUTS,
  variant = "default",
} = {}) {
  const resolvedCoreRoot = path.resolve(coreRoot);
  const hash = crypto.createHash("sha256");
  hash.update(`artifact:web-dist:v${ARTIFACT_VERSION}\n`);
  hash.update(`variant:${variant}\n`);
  hash.update(`appVersion:${appVersion}\n`);
  hash.update(`${JSON.stringify(extra, Object.keys(extra).sort())}\n`);
  for (const input of inputs) {
    const relativeInput = trimValue(input);
    if (!relativeInput) {
      continue;
    }
    walkForFingerprint({
      hash,
      rootDir: resolvedCoreRoot,
      currentRelativePath: relativeInput,
    });
  }
  return hash.digest("hex").slice(0, 20);
}

function resolveWebDistArtifactRoot({
  coreRoot,
  env = process.env,
  appVersion = "",
  extra = {},
  inputs = DEFAULT_INPUTS,
  variant = "default",
} = {}) {
  const resolvedCoreRoot = path.resolve(coreRoot);
  const layout = buildCtxCacheEnv({
    cwd: resolvedCoreRoot,
    env,
    mode: "workspace",
  }).layout;
  const cacheKey = computeWebDistCacheKey({
    coreRoot: resolvedCoreRoot,
    appVersion,
    extra,
    inputs,
    variant,
  });
  const artifactRoot = path.join(layout.artifactsDir, "web-dist", cacheKey);
  return {
    artifactRoot,
    cacheKey,
    layout,
  };
}

function resolveWebDistArtifactDir(options = {}) {
  return path.join(resolveWebDistArtifactRoot(options).artifactRoot, "dist");
}

function artifactMetadataPath(artifactRoot) {
  return path.join(artifactRoot, ARTIFACT_MARKER);
}

function hasReusableWebDistArtifact(artifactRoot) {
  return fs.existsSync(path.join(artifactRoot, "dist"))
    && fs.existsSync(artifactMetadataPath(artifactRoot));
}

function writeArtifactMetadata({
  artifactRoot,
  appVersion,
  cacheKey,
  extra,
  variant,
}) {
  const payload = {
    version: ARTIFACT_VERSION,
    artifact: "web-dist",
    variant,
    cache_key: cacheKey,
    app_version: appVersion,
    extra,
    created_at: new Date().toISOString(),
  };
  fs.writeFileSync(artifactMetadataPath(artifactRoot), `${JSON.stringify(payload, null, 2)}\n`, "utf8");
}

function buildBazelCommandContext({ coreRoot, env = process.env } = {}) {
  const resolvedCoreRoot = path.resolve(coreRoot);
  const repoRoot = path.resolve(resolvedCoreRoot, "..");
  const layout = resolveCtxCacheLayout({ cwd: resolvedCoreRoot, env });
  fs.mkdirSync(layout.tmpDir, { recursive: true });
  fs.mkdirSync(layout.bazelOutputUserRoot, { recursive: true });
  fs.mkdirSync(layout.bazelDiskCacheDir, { recursive: true });
  fs.mkdirSync(layout.bazelRepositoryCacheDir, { recursive: true });
  const bazelEnv = {
    ...env,
    BUILD_WORKSPACE_DIRECTORY: String(env.BUILD_WORKSPACE_DIRECTORY || repoRoot),
    TMPDIR: layout.tmpDir,
  };
  const directRunBazelVersion = resolveDirectRunBazelVersion({ repoRoot, env: bazelEnv });
  if (directRunBazelVersion) {
    bazelEnv.USE_BAZEL_VERSION = directRunBazelVersion;
  }
  return {
    bazelBinary: bazeliskBinaryPath({ repoRoot, env: bazelEnv }),
    bazelCommandArgs: [
      `--disk_cache=${layout.bazelDiskCacheDir}`,
      `--repository_cache=${layout.bazelRepositoryCacheDir}`,
    ],
    bazelEnv,
    repoRoot,
    startupArgs: [`--output_user_root=${layout.bazelOutputUserRoot}`],
  };
}

function resolveDirectRunBazelVersion({
  repoRoot,
  env = process.env,
  fileExists = fs.existsSync,
  readFile = fs.readFileSync,
} = {}) {
  if (trimValue(env.USE_BAZEL_VERSION)) {
    return trimValue(env.USE_BAZEL_VERSION);
  }
  const bazelVersionPath = path.join(repoRoot, BAZEL_VERSION_FILE);
  if (!fileExists(bazelVersionPath)) {
    return "";
  }
  const versionLines = String(readFile(bazelVersionPath, "utf8") || "")
    .split(/\r?\n/u)
    .map((line) => trimValue(line))
    .filter(Boolean);
  if (versionLines.length < 2) {
    return "";
  }
  if (!versionLines[0].startsWith("buildbuddy-io/")) {
    return "";
  }
  return versionLines[1];
}

function runChecked(command, args, options, failureMessage, spawnSyncImpl = childProcess.spawnSync) {
  const result = spawnSyncImpl(command, args, options);
  if (result.error) {
    throw result.error;
  }
  if (typeof result.status === "number" && result.status !== 0) {
    throw new Error(failureMessage || `${command} ${args.join(" ")} failed with status ${result.status}`);
  }
  if (result.signal) {
    throw new Error(failureMessage || `${command} ${args.join(" ")} terminated with signal ${result.signal}`);
  }
  return result;
}

function runWebDistBuild({
  coreRoot,
  env = process.env,
  destinationDir,
  targetLabel = WEB_DIST_SYNC_TARGET,
  spawnSyncImpl = childProcess.spawnSync,
  withHostJobBudgetImpl = withHostJobBudget,
} = {}) {
  const resolvedCoreRoot = path.resolve(coreRoot);
  const resolvedDestinationDir = path.resolve(destinationDir || path.join(resolvedCoreRoot, "apps", "web", "dist"));
  const { bazelBinary, bazelCommandArgs, bazelEnv, repoRoot, startupArgs } = buildBazelCommandContext({
    coreRoot: resolvedCoreRoot,
    env,
  });

  withHostJobBudgetImpl({
    budgetKey: HOST_HEAVY_BUDGET_KEY,
    command: `bazel run ${targetLabel}`,
    cwd: repoRoot,
    env: bazelEnv,
  }, () => {
    runChecked(
      bazelBinary,
      [
        ...startupArgs,
        "run",
        ...bazelCommandArgs,
        ...buildBuildBuddyAuthArgs(bazelEnv),
        targetLabel,
        "--",
        resolvedDestinationDir,
      ],
      {
        cwd: repoRoot,
        env: bazelEnv,
        stdio: "inherit",
      },
      `bazel run ${targetLabel} for web dist failed`,
      spawnSyncImpl,
    );
  });

  if (!fs.existsSync(resolvedDestinationDir)) {
    throw new Error(`Bazel web dist target did not materialize ${resolvedDestinationDir}`);
  }
  return resolvedDestinationDir;
}

function ensureWebDistArtifact({
  coreRoot,
  env = process.env,
  appVersion = "",
  extra = {},
  inputs = DEFAULT_INPUTS,
  runWebDistBuildImpl = runWebDistBuild,
  variant = "default",
} = {}) {
  const resolvedCoreRoot = path.resolve(coreRoot);
  const { env: buildEnv, layout } = buildCtxCacheEnv({
    cwd: resolvedCoreRoot,
    env,
    mode: "workspace",
    mkdir: true,
  });
  const { artifactRoot, cacheKey } = resolveWebDistArtifactRoot({
    coreRoot: resolvedCoreRoot,
    env: buildEnv,
    appVersion,
    extra,
    inputs,
    variant,
  });

  if (hasReusableWebDistArtifact(artifactRoot)) {
    return {
      artifactRoot,
      cacheKey,
      distDir: path.join(artifactRoot, "dist"),
      reused: true,
    };
  }

  if (fs.existsSync(artifactRoot)) {
    fs.rmSync(artifactRoot, { recursive: true, force: true });
  }

  const tmpArtifactRoot = path.join(
    layout.tmpDir,
    `ctx-web-dist-${variant}-${process.pid}-${crypto.randomBytes(4).toString("hex")}`,
  );
  const distDir = path.join(tmpArtifactRoot, "dist");
  fs.mkdirSync(tmpArtifactRoot, { recursive: true });

  runWebDistBuildImpl({
    coreRoot: resolvedCoreRoot,
    destinationDir: distDir,
    env: {
      ...buildEnv,
      ...(trimValue(appVersion) ? { VITE_CTX_APP_VERSION: appVersion } : {}),
    },
  });
  if (!fs.existsSync(distDir)) {
    throw new Error(`Bazel web dist build did not materialize ${distDir}`);
  }

  writeArtifactMetadata({
    artifactRoot: tmpArtifactRoot,
    appVersion,
    cacheKey,
    extra,
    variant,
  });
  fs.mkdirSync(path.dirname(artifactRoot), { recursive: true });
  try {
    fs.renameSync(tmpArtifactRoot, artifactRoot);
  } catch (error) {
    if (error && (error.code === "EEXIST" || error.code === "ENOTEMPTY")) {
      fs.rmSync(tmpArtifactRoot, { recursive: true, force: true });
      if (hasReusableWebDistArtifact(artifactRoot)) {
        return {
          artifactRoot,
          cacheKey,
          distDir: path.join(artifactRoot, "dist"),
          reused: true,
        };
      }
    }
    throw error;
  }

  return {
    artifactRoot,
    cacheKey,
    distDir: path.join(artifactRoot, "dist"),
    reused: false,
  };
}

function resolveDesktopWebDistSource(coreRoot, env = process.env) {
  const configured = trimValue(env.CTX_DESKTOP_WEB_DIST);
  if (configured) {
    return resolveConfiguredPath(configured, { cwd: coreRoot });
  }
  return path.join(coreRoot, "apps", "web", "dist");
}

module.exports = {
  ARTIFACT_MARKER,
  ARTIFACT_VERSION,
  DEFAULT_INPUTS,
  WEB_DIST_SYNC_TARGET,
  buildBazelCommandContext,
  computeWebDistCacheKey,
  ensureWebDistArtifact,
  hasReusableWebDistArtifact,
  resolveDirectRunBazelVersion,
  resolveDesktopWebDistSource,
  resolveWebDistArtifactDir,
  resolveWebDistArtifactRoot,
  runWebDistBuild,
};
