#!/usr/bin/env node

const fs = require("node:fs");
const path = require("node:path");

const {
  GENERATED_PACKAGE_SCRIPT_NAMES,
  GENERATED_PACKAGE_SCRIPT_PREFIXES,
  GENERATED_TURBO_TASK_PREFIXES,
  buildGeneratedPackageScripts,
  buildGeneratedTurboTasks,
  buildWorkspaceGraph,
} = require("./lib/rust_workspace_graph.cjs");

function parseArgs(argv) {
  return {
    check: argv.includes("--check"),
  };
}

function sortObjectEntries(object) {
  return Object.fromEntries(Object.entries(object));
}

function stripGeneratedKeys(object, prefixes, exactNames) {
  const next = {};
  for (const [key, value] of Object.entries(object)) {
    const isGeneratedPrefix = prefixes.some((prefix) => key.startsWith(prefix));
    const isGeneratedExact = exactNames && exactNames.has(key);
    if (isGeneratedPrefix || isGeneratedExact) {
      continue;
    }
    next[key] = value;
  }
  return next;
}

function updatePackageJson(coreRoot, graph) {
  const packageJsonPath = path.join(coreRoot, "package.json");
  const packageJson = JSON.parse(fs.readFileSync(packageJsonPath, "utf8"));
  const preservedScripts = stripGeneratedKeys(
    packageJson.scripts || {},
    GENERATED_PACKAGE_SCRIPT_PREFIXES,
    GENERATED_PACKAGE_SCRIPT_NAMES,
  );
  packageJson.scripts = sortObjectEntries({
    ...preservedScripts,
    ...buildGeneratedPackageScripts(graph),
  });
  return {
    path: packageJsonPath,
    content: `${JSON.stringify(packageJson, null, 2)}\n`,
  };
}

function updateTurboJson(coreRoot, graph) {
  const turboJsonPath = path.join(coreRoot, "turbo.json");
  const turboJson = JSON.parse(fs.readFileSync(turboJsonPath, "utf8"));
  const preservedTasks = stripGeneratedKeys(
    turboJson.tasks || {},
    GENERATED_TURBO_TASK_PREFIXES,
    new Set(),
  );
  turboJson.tasks = sortObjectEntries({
    ...preservedTasks,
    ...buildGeneratedTurboTasks(graph),
  });
  return {
    path: turboJsonPath,
    content: `${JSON.stringify(turboJson, null, 2)}\n`,
  };
}

function main() {
  const args = parseArgs(process.argv.slice(2));
  const coreRoot = path.resolve(__dirname, "..");
  const graph = buildWorkspaceGraph(coreRoot);
  const updates = [updatePackageJson(coreRoot, graph), updateTurboJson(coreRoot, graph)];
  let hasDiff = false;

  for (const update of updates) {
    const current = fs.readFileSync(update.path, "utf8");
    if (current !== update.content) {
      hasDiff = true;
      if (!args.check) {
        fs.writeFileSync(update.path, update.content);
      }
    }
  }

  if (args.check && hasDiff) {
    console.error("Rust Turbo task graph is out of date. Run `pnpm rust:turbo:sync`.");
    process.exit(1);
  }
}

main();
