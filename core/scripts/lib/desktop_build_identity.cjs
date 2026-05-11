const childProcess = require("node:child_process");
const path = require("node:path");

const { readDesktopVersion } = require("../desktop_version.cjs");
const { assertValidVersion } = require("../desktop_set_version.cjs");

const DEFAULT_CHANNEL = "stable";
const RELEASE_CHANNELS = new Set(["stable", "canary", "preview", "e2e"]);
const BUILD_MODES = new Set(["dev", "packaged", "release", "e2e"]);

function trimmedEnv(env, name) {
  const value = String(env?.[name] || "").trim();
  return value || "";
}

function normalizeReleaseChannel(value) {
  const normalized = String(value || "").trim() || DEFAULT_CHANNEL;
  if (!RELEASE_CHANNELS.has(normalized)) {
    throw new Error(`unsupported release channel '${value}'`);
  }
  return normalized;
}

function normalizeReleaseCommit(value) {
  const normalized = String(value || "").trim().toLowerCase();
  if (!normalized) {
    return "";
  }
  if (!/^[0-9a-f]{7,64}$/.test(normalized)) {
    throw new Error(`invalid release source commit '${value}'`);
  }
  return normalized;
}

function stripPreReleaseAndBuild(version) {
  return String(version || "").trim().replace(/[-+].*$/, "");
}

function deriveCiReleaseVersion({ baseVersion, channel, sourceCommit }) {
  const normalizedBaseVersion = assertValidVersion(baseVersion);
  const normalizedChannel = normalizeReleaseChannel(channel);
  if (normalizedChannel === "stable") {
    return normalizedBaseVersion;
  }
  const normalizedSourceCommit = normalizeReleaseCommit(sourceCommit);
  if (!normalizedSourceCommit) {
    throw new Error(`release source commit is required for ${normalizedChannel} versions`);
  }
  const stableBaseVersion = stripPreReleaseAndBuild(normalizedBaseVersion);
  return `${stableBaseVersion}-${normalizedChannel}.${normalizedSourceCommit.slice(0, 12)}`;
}

function normalizeBuildMode(value) {
  const normalized = String(value || "").trim();
  if (!normalized) {
    return "";
  }
  if (!BUILD_MODES.has(normalized)) {
    throw new Error(`unsupported desktop build mode '${value}'`);
  }
  return normalized;
}

function resolveBuildMode({ env = process.env, requestedMode = "" } = {}) {
  const explicit = normalizeBuildMode(requestedMode || trimmedEnv(env, "CTX_DESKTOP_BUILD_MODE"));
  if (explicit) {
    return explicit;
  }
  const channel = normalizeReleaseChannel(trimmedEnv(env, "RELEASE_CHANNEL"));
  if (channel === "e2e") {
    return "e2e";
  }
  if (channel !== "stable" || trimmedEnv(env, "RELEASE_VERSION") || trimmedEnv(env, "RELEASE_SOURCE_COMMIT")) {
    return "release";
  }
  return "dev";
}

function runGit(args, cwd) {
  const result = childProcess.spawnSync("git", ["-C", cwd, ...args], {
    encoding: "utf8",
    stdio: ["ignore", "pipe", "ignore"],
  });
  if (result.status !== 0) {
    return "";
  }
  return String(result.stdout || "").trim();
}

function gitRepoRoot(cwd) {
  return runGit(["rev-parse", "--show-toplevel"], cwd);
}

function findCargoWorkspaceRoot(cwd) {
  let current = path.resolve(cwd);
  for (;;) {
    const cargoToml = path.join(current, "Cargo.toml");
    try {
      const raw = require("node:fs").readFileSync(cargoToml, "utf8");
      if (raw.includes("[workspace]")) {
        return current;
      }
    } catch {}
    const next = path.dirname(current);
    if (next === current) {
      return "";
    }
    current = next;
  }
}

function sharedDevInstanceRoot(cwd, env = process.env) {
  const gitRoot = gitRepoRoot(cwd);
  const defaultRoot = gitRoot || findCargoWorkspaceRoot(cwd) || cwd;
  const rawOverride = trimmedEnv(env, "CTX_DEV_INSTANCE_ROOT");
  if (!rawOverride) {
    return defaultRoot;
  }
  if (path.isAbsolute(rawOverride)) {
    return rawOverride;
  }
  return path.join(defaultRoot, rawOverride);
}

function canonicalizeBestEffort(targetPath) {
  try {
    return require("node:fs").realpathSync.native(targetPath);
  } catch {
    return path.resolve(targetPath);
  }
}

function fnv1a64(bytes) {
  let hash = 0xcbf29ce484222325n;
  for (const byte of bytes) {
    hash ^= BigInt(byte);
    hash = (hash * 0x100000001b3n) & 0xffffffffffffffffn;
  }
  return hash;
}

function stableDevCompatibilityToken(cwd, env = process.env) {
  const resolvedRoot = canonicalizeBestEffort(sharedDevInstanceRoot(cwd, env));
  const key = process.platform === "win32"
    ? resolvedRoot.toLowerCase()
    : resolvedRoot;
  const hash = fnv1a64(Buffer.from(key, "utf8"));
  return `dev-${hash.toString(16).padStart(16, "0")}`;
}

function gitHeadBuildId(cwd) {
  const head = runGit(["rev-parse", "--short=12", "HEAD"], cwd);
  if (!head) {
    return "";
  }
  const dirty = runGit(["status", "--porcelain", "--untracked-files=no"], cwd);
  return dirty ? `${head}-dirty` : head;
}

function resolveEffectiveReleaseVersion({ coreRoot = path.resolve(__dirname, "../.."), env = process.env } = {}) {
  const explicitVersion = trimmedEnv(env, "CTX_RELEASE_EFFECTIVE_VERSION") || trimmedEnv(env, "RELEASE_VERSION");
  if (explicitVersion) {
    return assertValidVersion(explicitVersion);
  }
  const checkedInVersion = readDesktopVersion(coreRoot);
  const channel = normalizeReleaseChannel(trimmedEnv(env, "RELEASE_CHANNEL"));
  if (channel === "stable") {
    return checkedInVersion;
  }
  return deriveCiReleaseVersion({
    baseVersion: checkedInVersion,
    channel,
    sourceCommit: trimmedEnv(env, "RELEASE_SOURCE_COMMIT"),
  });
}

function resolveDesktopBuildIdentity({
  coreRoot = path.resolve(__dirname, "../.."),
  env = process.env,
  mode = "",
} = {}) {
  const normalizedMode = resolveBuildMode({ env, requestedMode: mode });
  const checkedInVersion = readDesktopVersion(coreRoot);
  const exactVersion = resolveEffectiveReleaseVersion({ coreRoot, env });
  const sourceCommit = normalizeReleaseCommit(trimmedEnv(env, "RELEASE_SOURCE_COMMIT"));
  const channel = normalizeReleaseChannel(trimmedEnv(env, "RELEASE_CHANNEL"));
  const explicitBuildId = trimmedEnv(env, "CTX_BUILD_ID");
  const buildId = explicitBuildId
    || sourceCommit.slice(0, 12)
    || gitHeadBuildId(coreRoot)
    || exactVersion;
  const explicitCompatibilityToken = trimmedEnv(env, "CTX_COMPATIBILITY_TOKEN");
  const compatibilityToken = explicitCompatibilityToken
    || (normalizedMode === "dev"
      ? trimmedEnv(env, "CTX_DEV_INSTANCE_ID") || stableDevCompatibilityToken(coreRoot, env)
      : `artifact-${sourceCommit || buildId}`);

  return {
    schemaVersion: 1,
    exactVersion,
    buildId,
    compatibilityToken,
    provenanceChannel: channel,
    sourceCommit: sourceCommit || null,
    mode: normalizedMode,
    checkedInVersion,
  };
}

function buildIdentityShellExports(identity) {
  const lines = [
    `CTX_RELEASE_EFFECTIVE_VERSION=${shellQuote(identity.exactVersion)}`,
    `CTX_BUILD_ID=${shellQuote(identity.buildId)}`,
    `CTX_COMPATIBILITY_TOKEN=${shellQuote(identity.compatibilityToken)}`,
  ];
  if (identity.mode === "dev") {
    lines.push(`CTX_DEV_INSTANCE_ID=${shellQuote(identity.compatibilityToken)}`);
  }
  return lines.join("\n");
}

function shellQuote(value) {
  return `'${String(value).replace(/'/g, `'\"'\"'`)}'`;
}

module.exports = {
  BUILD_MODES,
  DEFAULT_CHANNEL,
  RELEASE_CHANNELS,
  buildIdentityShellExports,
  deriveCiReleaseVersion,
  gitHeadBuildId,
  normalizeBuildMode,
  normalizeReleaseChannel,
  normalizeReleaseCommit,
  resolveBuildMode,
  resolveDesktopBuildIdentity,
  resolveEffectiveReleaseVersion,
  sharedDevInstanceRoot,
  stableDevCompatibilityToken,
  stripPreReleaseAndBuild,
};
