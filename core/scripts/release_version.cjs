#!/usr/bin/env node

const path = require("node:path");

const { readDesktopVersion } = require("./desktop_version.cjs");
const {
  DEFAULT_CHANNEL,
  deriveCiReleaseVersion,
  normalizeReleaseChannel,
  normalizeReleaseCommit,
  resolveDesktopBuildIdentity,
  resolveEffectiveReleaseVersion,
  stripPreReleaseAndBuild,
} = require("./lib/desktop_build_identity.cjs");

const stampEffectiveReleaseVersion = ({ coreRoot = path.resolve(__dirname, ".."), env = process.env } = {}) => {
  const checkedInVersion = readDesktopVersion(coreRoot);
  const effectiveVersion = resolveEffectiveReleaseVersion({ coreRoot, env });
  return {
    checkedInVersion,
    effectiveVersion,
    changed: false,
    identity: resolveDesktopBuildIdentity({
      coreRoot,
      env,
      mode: normalizeReleaseChannel(env.RELEASE_CHANNEL) === "stable" ? "release" : "",
    }),
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
  --stamp   Compatibility no-op; report the effective release version without mutating source files
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
  resolveDesktopBuildIdentity,
  stampEffectiveReleaseVersion,
  stripPreReleaseAndBuild,
};
