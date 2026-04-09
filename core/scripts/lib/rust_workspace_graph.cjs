const childProcess = require("node:child_process");
const path = require("node:path");

const {
  CTX_HTTP_SUITE_SCRIPT_INPUTS,
  getCtxHttpSuiteNames,
  getCtxHttpSuiteTaskName,
} = require("./ctx_http_suites.cjs");

const ROOT_RUST_INPUTS = [
  "Cargo.toml",
  "Cargo.lock",
  "rust-toolchain.toml",
  "rustfmt.toml",
  "clippy.toml",
  ".cargo/config.toml",
  "scripts/lib/cache_roots.cjs",
  "scripts/lib/ctx_http_suites.cjs",
  "scripts/lib/turbo_runner.cjs",
  "scripts/lib/rust_workspace_graph.cjs",
  "scripts/lib/rust_gate_plan.cjs",
  "scripts/ctx_http_suite_task.cjs",
  "scripts/rust_crate_task.cjs",
  "scripts/run_rust_gate.cjs",
  "scripts/run_rust_turbo.cjs",
  "scripts/sync_rust_turbo_tasks.cjs",
];

const GENERATED_PACKAGE_SCRIPT_PREFIXES = [
  "rust:crate:clippy:",
  "rust:crate:test:",
  "rust:crate:nextest:",
  "rust:ctx-http:test:",
];

const GENERATED_PACKAGE_SCRIPT_NAMES = new Set([
  "rust:turbo:sync",
  "rust:turbo:check",
]);

const GENERATED_TURBO_TASK_PREFIXES = [
  "rust:crate:clippy:",
  "rust:crate:test:",
  "rust:crate:nextest:",
  "rust:ctx-http:test:",
];

function runCargoMetadata(coreRoot) {
  const output = childProcess.execFileSync(
    "cargo",
    ["metadata", "--format-version", "1", "--no-deps"],
    {
      cwd: coreRoot,
      encoding: "utf8",
    },
  );
  return JSON.parse(output);
}

function normalizePathForMatch(value) {
  return String(value).replace(/\\/g, "/").replace(/\/+$/, "");
}

function buildWorkspaceGraph(coreRoot, metadata = runCargoMetadata(coreRoot)) {
  const workspaceMemberIds = new Set(metadata.workspace_members);
  const packages = metadata.packages.filter((pkg) => workspaceMemberIds.has(pkg.id));
  const packagesById = new Map(packages.map((pkg) => [pkg.id, pkg]));
  const packagesByDir = new Map();

  for (const pkg of packages) {
    const manifestDir = path.dirname(pkg.manifest_path);
    packagesByDir.set(path.resolve(manifestDir), pkg);
  }

  const crates = packages
    .map((pkg) => {
      const manifestDir = path.dirname(pkg.manifest_path);
      const relDir = normalizePathForMatch(path.relative(coreRoot, manifestDir));
      const deps = [];
      for (const dep of pkg.dependencies) {
        if (!dep.path) {
          continue;
        }
        const depDir = path.resolve(dep.path);
        const depPkg = packagesByDir.get(depDir);
        if (!depPkg) {
          continue;
        }
        deps.push(depPkg.name);
      }
      const uniqueDeps = [...new Set(deps)].sort();
      return {
        crateName: pkg.name,
        id: pkg.id,
        manifestPath: pkg.manifest_path,
        manifestDir,
        relDir,
        deps: uniqueDeps,
        reverseDeps: [],
        targets: pkg.targets.map((target) => ({
          name: target.name,
          kind: target.kind,
          srcPath: target.src_path,
        })),
      };
    })
    .sort((a, b) => a.crateName.localeCompare(b.crateName));

  const cratesByName = new Map(crates.map((crate) => [crate.crateName, crate]));
  for (const crate of crates) {
    for (const depName of crate.deps) {
      const dep = cratesByName.get(depName);
      if (!dep) {
        continue;
      }
      dep.reverseDeps.push(crate.crateName);
    }
  }
  for (const crate of crates) {
    crate.reverseDeps = [...new Set(crate.reverseDeps)].sort();
  }

  return {
    coreRoot,
    crates,
    cratesByName,
  };
}

function expandGraphNames(graph, startingNames, direction) {
  const queue = [...startingNames];
  const visited = new Set();
  while (queue.length > 0) {
    const crateName = queue.shift();
    if (!crateName || visited.has(crateName)) {
      continue;
    }
    const crate = graph.cratesByName.get(crateName);
    if (!crate) {
      continue;
    }
    visited.add(crateName);
    const neighbors = direction === "reverse" ? crate.reverseDeps : crate.deps;
    for (const neighbor of neighbors) {
      if (!visited.has(neighbor)) {
        queue.push(neighbor);
      }
    }
  }
  return [...visited].sort();
}

function expandDependencies(graph, crateNames) {
  return expandGraphNames(graph, crateNames, "forward");
}

function expandReverseDependencies(graph, crateNames) {
  return expandGraphNames(graph, crateNames, "reverse");
}

function getCrateByChangedPath(graph, changedPath) {
  const normalized = normalizePathForMatch(changedPath);
  for (const crate of graph.crates) {
    const prefix = normalizePathForMatch(`core/${crate.relDir}/`);
    if (normalized.startsWith(prefix)) {
      return crate.crateName;
    }
  }
  return null;
}

function collectChangedCrates(graph, changedPaths) {
  const crateNames = new Set();
  for (const changedPath of changedPaths) {
    const crateName = getCrateByChangedPath(graph, changedPath);
    if (crateName) {
      crateNames.add(crateName);
    }
  }
  return [...crateNames].sort();
}

function getClosureInputGlobs(graph, crateName) {
  const closure = expandDependencies(graph, [crateName]);
  const globs = [];
  for (const name of closure) {
    const crate = graph.cratesByName.get(name);
    if (crate) {
      globs.push(`${crate.relDir}/**`);
    }
  }
  return [...new Set([...ROOT_RUST_INPUTS, ...globs])];
}

function getDependencyInputGlobsWithoutSelf(graph, crateName) {
  const closure = expandDependencies(graph, [crateName]).filter((name) => name !== crateName);
  const globs = [];
  for (const name of closure) {
    const crate = graph.cratesByName.get(name);
    if (crate) {
      globs.push(`${crate.relDir}/**`);
    }
  }
  return [...new Set([...ROOT_RUST_INPUTS, ...globs])];
}

function getTurboClippyTaskName(crateName) {
  return `rust:crate:clippy:${crateName}`;
}

function getTurboTestTaskName(crateName) {
  return `rust:crate:test:${crateName}`;
}

function getTurboNextestTaskName(crateName) {
  return `rust:crate:nextest:${crateName}`;
}

function getCtxHttpSuiteInputGlobs(graph, suiteName) {
  const dependencyInputs = getDependencyInputGlobsWithoutSelf(graph, "ctx-http");
  const baseInputs = [
    ...dependencyInputs,
    ...CTX_HTTP_SUITE_SCRIPT_INPUTS,
    "crates/ctx-http/src/**",
  ];
  if (suiteName === "all") {
    return [...new Set([...baseInputs, "crates/ctx-http/tests/**"])];
  }
  if (suiteName === "base") {
    return [...new Set(baseInputs)];
  }

  const { getCtxHttpSuiteByName } = require("./ctx_http_suites.cjs");
  const suite = getCtxHttpSuiteByName(suiteName);
  if (!suite) {
    throw new Error(`unknown ctx-http suite: ${suiteName}`);
  }
  return [
    ...new Set([
      ...baseInputs,
      "crates/ctx-http/tests/common/**",
      ...suite.testFiles.map((testFile) => `crates/ctx-http/tests/${testFile}.rs`),
    ]),
  ];
}

function buildGeneratedPackageScripts(graph) {
  const scripts = {
    "rust:turbo:sync": "node scripts/sync_rust_turbo_tasks.cjs",
    "rust:turbo:check": "node scripts/sync_rust_turbo_tasks.cjs --check",
  };
  for (const crate of graph.crates) {
    scripts[getTurboClippyTaskName(crate.crateName)] =
      `node scripts/rust_crate_task.cjs --crate ${crate.crateName} --task clippy`;
    if (crate.crateName === "ctx-http") {
      scripts[getTurboTestTaskName(crate.crateName)] =
        "node scripts/ctx_http_suite_task.cjs --suite all";
    } else {
      scripts[getTurboTestTaskName(crate.crateName)] =
        `node scripts/rust_crate_task.cjs --crate ${crate.crateName} --task test`;
    }
    scripts[getTurboNextestTaskName(crate.crateName)] =
      `node scripts/rust_crate_task.cjs --crate ${crate.crateName} --task nextest`;
  }
  for (const suiteName of getCtxHttpSuiteNames({ includeAll: true })) {
    scripts[getCtxHttpSuiteTaskName(suiteName)] =
      `node scripts/ctx_http_suite_task.cjs --suite ${suiteName}`;
  }
  return scripts;
}

function buildGeneratedTurboTasks(graph) {
  const tasks = {};
  for (const crate of graph.crates) {
    const inputs = getClosureInputGlobs(graph, crate.crateName);
    tasks[getTurboClippyTaskName(crate.crateName)] = {
      inputs,
      outputs: [],
    };
    tasks[getTurboTestTaskName(crate.crateName)] = {
      inputs,
      outputs: [],
    };
    tasks[getTurboNextestTaskName(crate.crateName)] = {
      inputs,
      outputs: [],
    };
  }
  for (const suiteName of getCtxHttpSuiteNames({ includeAll: true })) {
    tasks[getCtxHttpSuiteTaskName(suiteName)] = {
      inputs: getCtxHttpSuiteInputGlobs(graph, suiteName),
      outputs: [],
    };
  }
  return tasks;
}

function getTurboTaskNamesForCrates(crateNames, taskKinds) {
  const taskNames = [];
  for (const crateName of [...crateNames].sort()) {
    for (const taskKind of taskKinds) {
      if (taskKind === "clippy") {
        taskNames.push(getTurboClippyTaskName(crateName));
      } else if (taskKind === "test") {
        if (crateName === "ctx-http") {
          taskNames.push(...getCtxHttpSuiteNames().map((suiteName) => getCtxHttpSuiteTaskName(suiteName)));
        } else {
          taskNames.push(getTurboTestTaskName(crateName));
        }
      } else if (taskKind === "nextest") {
        taskNames.push(getTurboNextestTaskName(crateName));
      } else {
        throw new Error(`unknown Rust task kind: ${taskKind}`);
      }
    }
  }
  return taskNames;
}

module.exports = {
  GENERATED_PACKAGE_SCRIPT_NAMES,
  GENERATED_PACKAGE_SCRIPT_PREFIXES,
  GENERATED_TURBO_TASK_PREFIXES,
  ROOT_RUST_INPUTS,
  buildGeneratedPackageScripts,
  buildGeneratedTurboTasks,
  buildWorkspaceGraph,
  collectChangedCrates,
  expandDependencies,
  expandReverseDependencies,
  getCtxHttpSuiteTaskName,
  getClosureInputGlobs,
  getCrateByChangedPath,
  getTurboClippyTaskName,
  getTurboNextestTaskName,
  getTurboTaskNamesForCrates,
  getTurboTestTaskName,
  normalizePathForMatch,
  runCargoMetadata,
};
