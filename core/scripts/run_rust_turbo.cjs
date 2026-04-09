#!/usr/bin/env node

const path = require("node:path");

const { buildCtxCacheEnv } = require("./lib/cache_roots.cjs");
const {
  buildWorkspaceGraph,
  collectChangedCrates,
  expandReverseDependencies,
  getTurboTaskNamesForCrates,
} = require("./lib/rust_workspace_graph.cjs");
const { runTurbo } = require("./lib/turbo_runner.cjs");

function parseArgs(argv) {
  const args = {
    all: false,
    changedFiles: [],
    crates: [],
    includeReverseDeps: false,
    mode: "workspace",
    tasks: [],
  };

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "--all") {
      args.all = true;
    } else if (arg === "--changed-file") {
      args.changedFiles.push(argv[index + 1] || "");
      index += 1;
    } else if (arg === "--crate") {
      args.crates.push(argv[index + 1] || "");
      index += 1;
    } else if (arg === "--include-reverse-deps") {
      args.includeReverseDeps = true;
    } else if (arg === "--mode") {
      args.mode = argv[index + 1] || args.mode;
      index += 1;
    } else if (arg === "--task") {
      args.tasks.push(argv[index + 1] || "");
      index += 1;
    } else {
      throw new Error(`unknown arg: ${arg}`);
    }
  }

  if (!args.all && args.changedFiles.length === 0 && args.crates.length === 0) {
    throw new Error("one of --all, --crate, or --changed-file is required");
  }
  if (args.tasks.length === 0) {
    throw new Error("at least one --task is required");
  }
  return args;
}

function resolveCrates(graph, args) {
  if (args.all) {
    return graph.crates.map((crate) => crate.crateName);
  }

  const directCrates = new Set(args.crates);
  for (const changedCrate of collectChangedCrates(graph, args.changedFiles)) {
    directCrates.add(changedCrate);
  }

  const resolved = [...directCrates].filter(Boolean).sort();
  if (!args.includeReverseDeps) {
    return resolved;
  }
  return expandReverseDependencies(graph, resolved);
}

function main() {
  const args = parseArgs(process.argv.slice(2));
  const coreRoot = path.resolve(__dirname, "..");
  const graph = buildWorkspaceGraph(coreRoot);
  const crateNames = resolveCrates(graph, args);
  if (crateNames.length === 0) {
    return;
  }

  const taskNames = getTurboTaskNamesForCrates(crateNames, args.tasks);
  const { env } = buildCtxCacheEnv({
    cwd: coreRoot,
    env: process.env,
    mode: args.mode,
    mkdir: true,
  });

  runTurbo({
    coreRoot,
    env,
    taskNames,
  });
}

main();
