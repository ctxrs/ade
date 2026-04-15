#!/usr/bin/env node

const childProcess = require("node:child_process");
const path = require("node:path");

const { resolveCtxCacheLayout } = require("./lib/cache_roots.cjs");
const { bazeliskBinaryPath, buildBuildBuddyAuthArgs } = require("./run_bazel_pilot.cjs");

const TARGET_LABEL = "//core/crates/codex-crp:codex-crp";
const TARGET_SPECS = Object.freeze({
  "darwin-aarch64": Object.freeze({
    platformLabel: "//tools/bazel/platforms:darwin_arm64",
    rustTarget: "aarch64-apple-darwin",
  }),
  "darwin-x86_64": Object.freeze({
    platformLabel: "//tools/bazel/platforms:darwin_x86_64",
    rustTarget: "x86_64-apple-darwin",
  }),
  "linux-aarch64": Object.freeze({
    platformLabel: "//tools/bazel/platforms:linux_arm64",
    rustTarget: "aarch64-unknown-linux-gnu",
  }),
  "linux-x86_64": Object.freeze({
    platformLabel: "//tools/bazel/platforms:linux_x86_64",
    rustTarget: "x86_64-unknown-linux-gnu",
  }),
});

function repoRoots() {
  const coreRoot = path.resolve(__dirname, "..");
  return {
    coreRoot,
    repoRoot: path.resolve(coreRoot, ".."),
  };
}

function parseArgs(argv) {
  const options = {
    targetKey: "",
  };
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "--target-key") {
      options.targetKey = String(argv[++index] || "").trim();
      continue;
    }
    throw new Error(`unsupported arg: ${arg}`);
  }
  if (!options.targetKey) {
    throw new Error("--target-key is required");
  }
  if (!TARGET_SPECS[options.targetKey]) {
    throw new Error(`unsupported codex-crp Bazel target '${options.targetKey}'`);
  }
  return options;
}

function buildBazelCommandContext(env = process.env) {
  const { coreRoot, repoRoot } = repoRoots();
  const layout = resolveCtxCacheLayout({ cwd: coreRoot, env });
  return {
    bazelBinary: bazeliskBinaryPath(),
    env: {
      ...env,
      TMPDIR: layout.tmpDir,
    },
    repoRoot,
    startupArgs: [`--output_user_root=${layout.bazelOutputUserRoot}`],
  };
}

function runChecked(command, args, options, failureMessage) {
  const result = childProcess.spawnSync(command, args, options);
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

function buildTargetBinary({ targetKey, env = process.env } = {}) {
  const targetSpec = TARGET_SPECS[targetKey];
  const { bazelBinary, env: bazelEnv, repoRoot, startupArgs } = buildBazelCommandContext(env);

  runChecked(
    bazelBinary,
    [
      ...startupArgs,
      "build",
      `--platforms=${targetSpec.platformLabel}`,
      ...buildBuildBuddyAuthArgs(bazelEnv),
      TARGET_LABEL,
    ],
    {
      cwd: repoRoot,
      env: bazelEnv,
      stdio: "inherit",
    },
    `bazel build ${TARGET_LABEL} for ${targetKey} failed`,
  );

  const result = runChecked(
    bazelBinary,
    [
      ...startupArgs,
      "cquery",
      `--platforms=${targetSpec.platformLabel}`,
      "--output=starlark",
      "--starlark:expr=\"\\n\".join([f.path for f in target.files.to_list()])",
      ...buildBuildBuddyAuthArgs(bazelEnv),
      TARGET_LABEL,
    ],
    {
      cwd: repoRoot,
      env: bazelEnv,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
    },
    `bazel cquery ${TARGET_LABEL} for ${targetKey} failed`,
  );

  const outputPaths = String(result.stdout || "")
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean);
  if (outputPaths.length !== 1) {
    throw new Error(`expected exactly one Bazel output for ${TARGET_LABEL} (${targetKey}), got ${outputPaths.length}`);
  }
  return path.resolve(repoRoot, outputPaths[0]);
}

function main() {
  const options = parseArgs(process.argv.slice(2));
  process.stdout.write(`${buildTargetBinary({ env: process.env, targetKey: options.targetKey })}\n`);
}

if (require.main === module) {
  main();
}

module.exports = {
  TARGET_LABEL,
  TARGET_SPECS,
  buildBazelCommandContext,
  buildTargetBinary,
  parseArgs,
  repoRoots,
};
