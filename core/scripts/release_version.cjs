#!/usr/bin/env node

const path = require("node:path");

const { readDesktopVersion } = require("./desktop_version.cjs");
const { assertValidVersion, setDesktopVersion } = require("./desktop_set_version.cjs");

const DEFAULT_CHANNEL = "stable";
const RELEASE_CHANNELS = new Set(["stable", "canary", "canary2", "e2e"]);

const normalizeReleaseChannel = (value) => {
  const normalized = String(value || "").trim() || DEFAULT_CHANNEL;
  if (!RELEASE_CHANNELS.has(normalized)) {
    throw new Error(`unsupported release channel '${normalized}'`);
  }
  return normalized;
};

const normalizeReleaseCommit = (value) => {
  const normalized = String(value || "").trim().toLowerCase();
  if (!normalized) {
    return "";
  }
  if (!/^[0-9a-f]{7,64}$/.test(normalized)) {
    throw new Error(`invalid release source commit '${value}'`);
  }
  return normalized;
};

const stripPreReleaseAndBuild = (version) => String(version || "").trim().replace(/[-+].*$/, "");

const deriveCiReleaseVersion = ({ baseVersion, channel, sourceCommit }) => {
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
};

const resolveEffectiveReleaseVersion = ({ coreRoot = path.resolve(__dirname, ".."), env = process.env } = {}) => {
  const explicitVersion = String(env.CTX_RELEASE_EFFECTIVE_VERSION || env.RELEASE_VERSION || "").trim();
  if (explicitVersion) {
    return assertValidVersion(explicitVersion);
  }
  const checkedInVersion = readDesktopVersion(coreRoot);
  const channel = normalizeReleaseChannel(env.RELEASE_CHANNEL);
  if (channel === "stable") {
    return checkedInVersion;
  }
  return deriveCiReleaseVersion({
    baseVersion: checkedInVersion,
    channel,
    sourceCommit: env.RELEASE_SOURCE_COMMIT,
  });
};

const stampEffectiveReleaseVersion = ({ coreRoot = path.resolve(__dirname, ".."), env = process.env } = {}) => {
  const checkedInVersion = readDesktopVersion(coreRoot);
  const effectiveVersion = resolveEffectiveReleaseVersion({ coreRoot, env });
  if (checkedInVersion !== effectiveVersion) {
    setDesktopVersion(effectiveVersion, { root: coreRoot });
  }
  return {
    checkedInVersion,
    effectiveVersion,
    changed: checkedInVersion !== effectiveVersion,
  };
};

const parseArgs = (argv) => {
  const options = {
    json: false,
    stamp: false,
  };
  for (const arg of argv) {
    if (arg === "--json") {
      options.json = true;
      continue;
    }
    if (arg === "--stamp") {
      options.stamp = true;
      continue;
    }
    if (arg === "--help" || arg === "-h") {
      options.help = true;
      continue;
    }
    throw new Error(`unsupported argument '${arg}'`);
  }
  return options;
};

const printUsage = () => {
  console.log(`usage: node core/scripts/release_version.cjs [--stamp] [--json]

options:
  --stamp   Update checked-out desktop version files to the effective release version
  --json    Print JSON instead of a bare version string
`);
};

const main = () => {
  const options = parseArgs(process.argv.slice(2));
  if (options.help) {
    printUsage();
    return;
  }
  const result = options.stamp
    ? stampEffectiveReleaseVersion()
    : {
      checkedInVersion: readDesktopVersion(path.resolve(__dirname, "..")),
      effectiveVersion: resolveEffectiveReleaseVersion(),
      changed: false,
    };

  if (options.json) {
    process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
    return;
  }
  process.stdout.write(result.effectiveVersion);
};

if (require.main === module) {
  try {
    main();
  } catch (error) {
    const detail = error instanceof Error ? error.message : String(error);
    console.error(`release_version failed: ${detail}`);
    process.exit(1);
  }
}

module.exports = {
  DEFAULT_CHANNEL,
  deriveCiReleaseVersion,
  normalizeReleaseChannel,
  normalizeReleaseCommit,
  resolveEffectiveReleaseVersion,
  stampEffectiveReleaseVersion,
  stripPreReleaseAndBuild,
};
