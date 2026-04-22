#!/usr/bin/env node

const path = require("node:path");

const config = require("./rust_bazel_deps.config.cjs");
const { syncGeneratedRustBazelDeps } = require("./lib/rust_bazel_deps.cjs");

function parseArgs(argv) {
  const options = {
    check: false,
    outPath: "",
  };
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "--check") {
      options.check = true;
      continue;
    }
    if (arg === "--out") {
      options.outPath = String(argv[++index] || "").trim();
      if (!options.outPath) {
        throw new Error("--out requires a path");
      }
      continue;
    }
    if (arg === "--help" || arg === "-h") {
      printHelp();
      process.exit(0);
    }
    throw new Error(`unsupported arg: ${arg}`);
  }
  return options;
}

function printHelp() {
  console.log("Usage: node scripts/sync_rust_bazel_deps.cjs [--check] [--out <path>]");
}

function main(argv = process.argv.slice(2)) {
  const options = parseArgs(argv);
  const coreRoot = path.resolve(__dirname, "..");
  const repoRoot = path.resolve(coreRoot, "..");
  const outPath = path.resolve(repoRoot, options.outPath || config.generatedOutputPath);
  const result = syncGeneratedRustBazelDeps({
    check: options.check,
    config,
    coreRoot,
    outPath,
    repoRoot,
  });
  if (options.check) {
    console.log(`generated Rust Bazel deps are current: ${path.relative(repoRoot, outPath)}`);
  } else if (result.wrote) {
    console.log(`wrote generated Rust Bazel deps: ${path.relative(repoRoot, outPath)}`);
  } else {
    console.log(`generated Rust Bazel deps already current: ${path.relative(repoRoot, outPath)}`);
  }
}

if (require.main === module) {
  try {
    main();
  } catch (error) {
    console.error(`error: ${error.message}`);
    process.exit(1);
  }
}

module.exports = {
  main,
  parseArgs,
};
