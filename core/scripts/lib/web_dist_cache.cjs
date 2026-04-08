const childProcess = require("node:child_process");
const crypto = require("node:crypto");
const fs = require("node:fs");
const path = require("node:path");

const { buildCtxCacheEnv, resolveConfiguredPath } = require("./cache_roots.cjs");

const ARTIFACT_VERSION = 1;
const ARTIFACT_MARKER = ".ctx-web-dist-artifact.json";
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

function resolveLocalNodeBin(packageRoot, tool) {
  const expectedBin = process.platform === "win32" ? `${tool}.cmd` : tool;
  let current = path.resolve(packageRoot);
  while (true) {
    const candidate = path.join(current, "node_modules", ".bin", expectedBin);
    if (fs.existsSync(candidate)) {
      return candidate;
    }
    const parent = path.dirname(current);
    if (parent === current) {
      break;
    }
    current = parent;
  }
  return path.join(path.resolve(packageRoot), "node_modules", ".bin", expectedBin);
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

function ensureWebDistArtifact({
  coreRoot,
  env = process.env,
  appVersion = "",
  extra = {},
  inputs = DEFAULT_INPUTS,
  resolveLocalNodeBinImpl = resolveLocalNodeBin,
  spawnSyncImpl = childProcess.spawnSync,
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

  const webRoot = path.join(resolvedCoreRoot, "apps", "web");
  const viteBin = resolveLocalNodeBinImpl(webRoot, "vite");
  if (!fs.existsSync(viteBin)) {
    throw new Error(`Missing local vite binary for ${webRoot}; run 'pnpm -C ${resolvedCoreRoot} install --frozen-lockfile'`);
  }

  const result = spawnSyncImpl(viteBin, ["build", "--outDir", distDir, "--emptyOutDir"], {
    cwd: webRoot,
    env: {
      ...buildEnv,
      ...(trimValue(appVersion) ? { VITE_CTX_APP_VERSION: appVersion } : {}),
    },
    stdio: "inherit",
  });
  if (result.error) {
    throw result.error;
  }
  if (result.status !== 0) {
    throw new Error(`vite build failed for cached web dist (${variant})`);
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
  computeWebDistCacheKey,
  ensureWebDistArtifact,
  hasReusableWebDistArtifact,
  resolveDesktopWebDistSource,
  resolveWebDistArtifactDir,
  resolveWebDistArtifactRoot,
};
