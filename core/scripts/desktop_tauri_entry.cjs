#!/usr/bin/env node

const childProcess = require("node:child_process");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const { HOST_HEAVY_BUDGET_KEY, withHostJobBudget } = require("./lib/host_job_budget.cjs");
const { resolveDesktopBuildIdentity } = require("./lib/desktop_build_identity.cjs");

const coreRoot = path.resolve(__dirname, "..");
const desktopAppRoot = path.join(coreRoot, "apps", "desktop");
const localTauriBin = path.join(desktopAppRoot, "node_modules", ".bin", "tauri");

function fail(message) {
  console.error(`desktop_tauri_entry failed: ${message}`);
  process.exit(1);
}

function parseArgs(argv) {
  const command = String(argv[2] || "").trim();
  const rawTauriArgs = argv.slice(3);
  const tauriArgs = rawTauriArgs[0] === "--" ? rawTauriArgs.slice(1) : rawTauriArgs;
  if (!command) {
    fail("missing command (expected build or dev)");
  }
  if (!["build", "dev"].includes(command)) {
    fail(`unsupported command '${command}' (expected build or dev)`);
  }
  return { command, tauriArgs };
}

function resolvePrepMode({ command, tauriArgs }) {
  if (command === "dev") {
    return "dev";
  }
  return tauriArgs.includes("--debug") ? "debug-build" : "release-build";
}

function shouldSkipPrep(env = process.env) {
  return String(env.CTX_DESKTOP_SKIP_PREP || "").trim() === "1";
}

function resolveTauriBudgetKey(command) {
  return String(command || "").trim() === "build" ? HOST_HEAVY_BUDGET_KEY : "";
}

function createTauriIdentityOverride({ command, prepMode, env = process.env }) {
  if (command !== "build") {
    return null;
  }
  const requestedMode = prepMode === "release-build"
    ? ""
    : prepMode === "debug-build"
      ? "packaged"
      : "dev";
  const identity = resolveDesktopBuildIdentity({
    coreRoot,
    env,
    mode: requestedMode,
  });
  const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-tauri-identity-"));
  const configPath = path.join(tempDir, "tauri.identity.json");
  fs.writeFileSync(configPath, `${JSON.stringify({ version: identity.exactVersion }, null, 2)}\n`, "utf8");
  return {
    configPath,
    tempDir,
    identity,
  };
}

function createInvocation(argv = process.argv, env = process.env) {
  const parsed = parseArgs(argv);
  const skipPrep = shouldSkipPrep(env);
  const prepMode = resolvePrepMode(parsed);
  const tauriIdentityOverride = createTauriIdentityOverride({
    command: parsed.command,
    prepMode,
    env,
  });
  return {
    ...parsed,
    prepMode,
    prepCommand: skipPrep ? "" : "node",
    prepArgs: skipPrep ? [] : ["scripts/desktop_prepare.cjs", "--mode", prepMode],
    skipPrep,
    tauriBudgetKey: resolveTauriBudgetKey(parsed.command),
    tauriCommand: resolveTauriCommand(),
    tauriExecArgs: tauriIdentityOverride
      ? [parsed.command, "--config", tauriIdentityOverride.configPath, ...parsed.tauriArgs]
      : [parsed.command, ...parsed.tauriArgs],
    tauriEnv: {
      ...normalizeTauriCliEnv(env),
      ...(tauriIdentityOverride?.identity
        ? {
          CTX_RELEASE_EFFECTIVE_VERSION: tauriIdentityOverride.identity.exactVersion,
          CTX_BUILD_ID: tauriIdentityOverride.identity.buildId,
          CTX_COMPATIBILITY_TOKEN: tauriIdentityOverride.identity.compatibilityToken,
          CTX_DEV_INSTANCE_ID: tauriIdentityOverride.identity.compatibilityToken,
        }
        : {}),
    },
    tauriIdentityOverride,
  };
}

function resolveTauriCommand() {
  if (!path.isAbsolute(localTauriBin)) {
    throw new Error(`expected absolute tauri path, got ${localTauriBin}`);
  }
  return localTauriBin;
}

function normalizeTauriCliEnv(env = process.env) {
  const normalizedEnv = { ...env };
  if (String(normalizedEnv.CI || "").trim() === "1") {
    normalizedEnv.CI = "true";
  }
  return normalizedEnv;
}

function run(
  command,
  args,
  {
    budgetKey = "",
    env = process.env,
    spawnSyncImpl = childProcess.spawnSync,
    withHostJobBudgetImpl = withHostJobBudget,
  } = {},
) {
  const invoke = () => {
    const result = spawnSyncImpl(command, args, {
      cwd: command === "node" ? coreRoot : desktopAppRoot,
      stdio: "inherit",
      env,
    });
    if (result.status !== 0) {
      process.exit(result.status ?? 1);
    }
  };
  if (!budgetKey) {
    invoke();
    return;
  }
  withHostJobBudgetImpl({
    budgetKey,
    command: `${command} ${args.join(" ")}`.trim(),
    cwd: command === "node" ? coreRoot : desktopAppRoot,
    env,
  }, invoke);
}

function main(
  argv = process.argv,
  {
    spawnSyncImpl = childProcess.spawnSync,
    withHostJobBudgetImpl = withHostJobBudget,
  } = {},
) {
  const invocation = createInvocation(argv);
  if (!invocation.skipPrep) {
    run(invocation.prepCommand, invocation.prepArgs, {
      spawnSyncImpl,
      withHostJobBudgetImpl,
    });
  }
  try {
    run(invocation.tauriCommand, invocation.tauriExecArgs, {
      env: invocation.tauriEnv,
      budgetKey: invocation.tauriBudgetKey,
      spawnSyncImpl,
      withHostJobBudgetImpl,
    });
  } finally {
    if (invocation.tauriIdentityOverride?.tempDir) {
      fs.rmSync(invocation.tauriIdentityOverride.tempDir, { recursive: true, force: true });
    }
  }
}

if (require.main === module) {
  main();
}

module.exports = {
  createInvocation,
  main,
  normalizeTauriCliEnv,
  parseArgs,
  resolvePrepMode,
  resolveTauriBudgetKey,
  shouldSkipPrep,
};
