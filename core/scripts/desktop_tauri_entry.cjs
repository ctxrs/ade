#!/usr/bin/env node

const childProcess = require("node:child_process");
const path = require("node:path");

const coreRoot = path.resolve(__dirname, "..");

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
    tauriCommand: "pnpm",
    tauriExecArgs: ["-C", "apps/desktop", "exec", "tauri", parsed.command, ...parsed.tauriArgs],
  };
}

function run(command, args) {
  const result = childProcess.spawnSync(command, args, {
    cwd: coreRoot,
    stdio: "inherit",
    env: process.env,
  });
  if (result.status !== 0) {
    process.exit(result.status ?? 1);
  }
}

function main(argv = process.argv) {
  const invocation = createInvocation(argv);
  run(invocation.prepCommand, invocation.prepArgs);
  run(invocation.tauriCommand, invocation.tauriExecArgs);
}

if (require.main === module) {
  main();
}

module.exports = {
  createInvocation,
  main,
  parseArgs,
  resolvePrepMode,
};
