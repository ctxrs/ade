#!/usr/bin/env node

const childProcess = require("node:child_process");
const path = require("node:path");

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

function createInvocation(argv = process.argv) {
  const parsed = parseArgs(argv);
  return {
    ...parsed,
    prepMode: resolvePrepMode(parsed),
    prepCommand: "node",
    prepArgs: ["scripts/desktop_prepare.cjs", "--mode", resolvePrepMode(parsed)],
    tauriCommand: resolveTauriCommand(),
    tauriExecArgs: [parsed.command, ...parsed.tauriArgs],
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

function run(command, args, { env = process.env } = {}) {
  const result = childProcess.spawnSync(command, args, {
    cwd: command === "node" ? coreRoot : desktopAppRoot,
    stdio: "inherit",
    env,
  });
  if (result.status !== 0) {
    process.exit(result.status ?? 1);
  }
}

function main(argv = process.argv) {
  const invocation = createInvocation(argv);
  run(invocation.prepCommand, invocation.prepArgs);
  run(invocation.tauriCommand, invocation.tauriExecArgs, {
    env: normalizeTauriCliEnv(process.env),
  });
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
};
