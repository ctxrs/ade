#!/usr/bin/env node

const childProcess = require("node:child_process");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const { HOST_HEAVY_BUDGET_KEY, withHostJobBudget } = require("./lib/host_job_budget.cjs");
const { resolveDesktopBuildIdentity } = require("./lib/desktop_build_identity.cjs");

const coreRoot = path.resolve(__dirname, "..");
const desktopAppRoot = path.join(coreRoot, "apps", "desktop");
const desktopTauriConfigPath = path.join(desktopAppRoot, "src-tauri", "tauri.conf.json");
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
  const config = {
    version: identity.exactVersion,
    ...createExtraBundleResourcesConfig(env),
  };
  fs.writeFileSync(configPath, `${JSON.stringify(config, null, 2)}\n`, "utf8");
  return {
    configPath,
    tempDir,
    identity,
  };
}

function createExtraBundleResourcesConfig(env = process.env) {
  const extraResources = parseExtraBundleResources(env);
  if (extraResources.length === 0) {
    return {};
  }
  return {
    bundle: {
      resources: mergeBundleResources(readBaseBundleResources(), extraResources),
    },
  };
}

function parseExtraBundleResources(env = process.env) {
  const raw = String(env.CTX_TAURI_EXTRA_BUNDLE_RESOURCES_JSON || "").trim();
  if (!raw) {
    return [];
  }
  let parsed;
  try {
    parsed = JSON.parse(raw);
  } catch (err) {
    throw new Error(
      `CTX_TAURI_EXTRA_BUNDLE_RESOURCES_JSON must be a JSON string array: ${err?.message ?? err}`,
    );
  }
  if (!Array.isArray(parsed)) {
    throw new Error("CTX_TAURI_EXTRA_BUNDLE_RESOURCES_JSON must be a JSON string array");
  }
  return parsed.map((entry, index) => {
    const value = String(entry || "").trim();
    if (!value) {
      throw new Error(`CTX_TAURI_EXTRA_BUNDLE_RESOURCES_JSON[${index}] must not be empty`);
    }
    return value;
  });
}

function readBaseBundleResources() {
  const config = JSON.parse(fs.readFileSync(desktopTauriConfigPath, "utf8"));
  const resources = config?.bundle?.resources;
  if (!Array.isArray(resources)) {
    throw new Error("tauri.conf.json bundle.resources must be an array");
  }
  return resources.map((entry, index) => {
    const value = String(entry || "").trim();
    if (!value) {
      throw new Error(`tauri.conf.json bundle.resources[${index}] must not be empty`);
    }
    return value;
  });
}

function mergeBundleResources(baseResources, extraResources) {
  return [...new Set([...baseResources, ...extraResources])];
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
  mergeBundleResources,
  main,
  normalizeTauriCliEnv,
  parseExtraBundleResources,
  parseArgs,
  resolvePrepMode,
  resolveTauriBudgetKey,
  shouldSkipPrep,
};
