#!/usr/bin/env node

const childProcess = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");

const { buildTargetBinary: buildCodexTargetBinary } = require("./codex_crp_bazel.cjs");
const { ensureCacheLayout, resolveCtxCacheLayout } = require("./lib/cache_roots.cjs");
const { bazeliskBinaryPath, buildBuildBuddyAuthArgs } = require("./run_bazel_pilot.cjs");

const PROVIDER_SPECS = Object.freeze({
  "acp-crp-bridge": Object.freeze({
    artifactKind: "binary",
    binaryName: "acp-crp-bridge",
    targetLabel: "//external-harnesses/acp-crp-bridge:acp-crp-bridge",
  }),
  amp: Object.freeze({
    artifactKind: "archive",
    targetLabel: "//harness-adapters/example-acp:provider-stage-archive",
  }),
  "claude-crp": Object.freeze({
    artifactKind: "archive",
    passTargetKeyToRun: true,
    targetLabel: "//external-harnesses/claude-crp:provider-stage-archive",
  }),
  codex: Object.freeze({
    artifactKind: "binary",
    resolver: "codex-crp-bazel",
  }),
  droid: Object.freeze({
    artifactKind: "binary",
    binaryName: "droid-acp",
    targetLabel: "//harness-adapters/droid-acp:droid-acp",
  }),
  goose: Object.freeze({
    artifactKind: "archive",
    passTargetKeyToRun: true,
    targetLabel: "//core/crates/ctx-provider-accounts:goose-provider-stage-archive",
  }),
  openhands: Object.freeze({
    artifactKind: "archive",
    passTargetKeyToRun: true,
    targetLabel: "//core/crates/ctx-provider-accounts:openhands-provider-stage-archive",
  }),
  pi: Object.freeze({
    artifactKind: "archive",
    targetLabel: "//harness-adapters/pi-acp:provider-stage-archive",
  }),
});

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
    providerId: "",
    targetKey: "",
  };
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "--provider-id") {
      options.providerId = String(argv[++index] || "").trim();
      continue;
    }
    if (arg === "--target-key") {
      options.targetKey = String(argv[++index] || "").trim();
      continue;
    }
    throw new Error(`unsupported arg: ${arg}`);
  }
  if (!options.providerId) {
    throw new Error("--provider-id is required");
  }
  if (!PROVIDER_SPECS[options.providerId]) {
    throw new Error(`unsupported provider-deps Bazel provider '${options.providerId}'`);
  }
  if (!options.targetKey) {
    throw new Error("--target-key is required");
  }
  if (!TARGET_SPECS[options.targetKey]) {
    throw new Error(`unsupported provider-deps Bazel target '${options.targetKey}'`);
  }
  return options;
}

function buildBazelCommandContext(env = process.env) {
  const { coreRoot, repoRoot } = repoRoots();
  const layout = resolveCtxCacheLayout({ cwd: coreRoot, env });
  ensureCacheLayout(layout, { includeTargets: false });
  return {
    bazelBinary: bazeliskBinaryPath(),
    env: {
      ...env,
      TMPDIR: layout.tmpDir,
      TMP: layout.tmpDir,
      TEMP: layout.tmpDir,
    },
    layout,
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

function runArchiveTarget({ env = process.env, providerSpec, targetKey = "" } = {}) {
  const { bazelBinary, env: bazelEnv, layout, repoRoot, startupArgs } = buildBazelCommandContext(env);
  const archiveOutDir = fs.mkdtempSync(path.join(layout.artifactsDir, "provider-deps-bazel-archive-"));
  const result = runChecked(
    bazelBinary,
    [
      ...startupArgs,
      "run",
      ...buildBuildBuddyAuthArgs(bazelEnv),
      providerSpec.targetLabel,
      ...(providerSpec.passTargetKeyToRun ? ["--", targetKey] : []),
    ],
    {
      cwd: repoRoot,
      env: {
        ...bazelEnv,
        CTX_PROVIDER_NODE_ARCHIVE_OUT_DIR: archiveOutDir,
        CTX_PROVIDER_CLAUDE_CRP_ARCHIVE_OUT_DIR: archiveOutDir,
      },
      encoding: "utf8",
      stdio: ["ignore", "pipe", "inherit"],
    },
    `bazel run ${providerSpec.targetLabel} failed`,
  );
  const outputPaths = String(result.stdout || "")
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean);
  if (outputPaths.length !== 1) {
    throw new Error(`expected exactly one archive path from ${providerSpec.targetLabel}, got ${outputPaths.length}`);
  }
  const archivePath = path.resolve(outputPaths[0]);
  if (!fs.existsSync(archivePath)) {
    throw new Error(`missing provider archive from ${providerSpec.targetLabel}: ${archivePath}`);
  }
  return archivePath;
}

function buildTargetArtifact({ env = process.env, providerId, targetKey } = {}) {
  const providerSpec = PROVIDER_SPECS[providerId];
  if (providerSpec.resolver === "codex-crp-bazel") {
    return buildCodexTargetBinary({ env, targetKey });
  }
  if (providerSpec.artifactKind === "archive") {
    return runArchiveTarget({ env, providerSpec, targetKey });
  }
  const targetSpec = TARGET_SPECS[targetKey];
  const { bazelBinary, env: bazelEnv, repoRoot, startupArgs } = buildBazelCommandContext(env);

  runChecked(
    bazelBinary,
    [
      ...startupArgs,
      "build",
      `--platforms=${targetSpec.platformLabel}`,
      ...buildBuildBuddyAuthArgs(bazelEnv),
      providerSpec.targetLabel,
    ],
    {
      cwd: repoRoot,
      env: bazelEnv,
      stdio: "inherit",
    },
    `bazel build ${providerSpec.targetLabel} for ${providerId} (${targetKey}) failed`,
  );

  const result = runChecked(
    bazelBinary,
    [
      ...startupArgs,
      "cquery",
      `--platforms=${targetSpec.platformLabel}`,
      "--output=starlark",
      '--starlark:expr="\\n".join([f.path for f in target.files.to_list()])',
      ...buildBuildBuddyAuthArgs(bazelEnv),
      providerSpec.targetLabel,
    ],
    {
      cwd: repoRoot,
      env: bazelEnv,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
    },
    `bazel cquery ${providerSpec.targetLabel} for ${providerId} (${targetKey}) failed`,
  );

  const outputPaths = String(result.stdout || "")
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean);
  if (outputPaths.length !== 1) {
    throw new Error(
      `expected exactly one Bazel output for ${providerSpec.targetLabel} (${providerId}/${targetKey}), got ${outputPaths.length}`,
    );
  }
  return path.resolve(repoRoot, outputPaths[0]);
}

function main() {
  const options = parseArgs(process.argv.slice(2));
  process.stdout.write(
    `${buildTargetArtifact({
      env: process.env,
      providerId: options.providerId,
      targetKey: options.targetKey,
    })}\n`,
  );
}

if (require.main === module) {
  main();
}

module.exports = {
  PROVIDER_SPECS,
  TARGET_SPECS,
  buildTargetArtifact,
  buildBazelCommandContext,
  parseArgs,
  repoRoots,
  runArchiveTarget,
};
