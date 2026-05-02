const childProcess = require("node:child_process");
const path = require("node:path");

const {
  CTX_HTTP_SHARED_SOURCE_GLOBS,
  CTX_HTTP_SUITE_SCRIPT_INPUTS,
  getCtxHttpSuiteNames,
  getCtxHttpSuiteNamesForAllTarget,
  getCtxHttpSuiteTaskName,
} = require("./ctx_http_suites.cjs");

const ROOT_RUST_INPUTS = [
  "Cargo.toml",
  "Cargo.Bazel.Cargo.lock",
  "Cargo.Bazel.lock",
  "Cargo.lock",
  "rust-toolchain.toml",
  "rustfmt.toml",
  ".cargo/config.toml",
  "scripts/lib/bazel_rust_targets.cjs",
  "scripts/lib/cache_roots.cjs",
  "scripts/lib/ctx_http_suites.cjs",
  "scripts/lib/rust_workspace_graph.cjs",
  "scripts/lib/rust_gate_plan.cjs",
  "scripts/ctx_http_suite_task.cjs",
  "scripts/rust_bazel_deps.config.cjs",
  "scripts/rust_crate_task.cjs",
  "scripts/run_rust_gate.cjs",
  "scripts/sync_rust_bazel_deps.cjs",
  "scripts/sync_rust_package_scripts.cjs",
];

const REPO_RUST_BUILD_GRAPH_INPUTS = [
  ".bazelignore",
  ".bazelrc",
  ".bazelversion",
  "BUILD.bazel",
  "MODULE.bazel",
  "MODULE.bazel.lock",
  "buildbuddy.yaml",
  "user.bazelrc.example",
];

const REPO_RUST_BUILD_GRAPH_INPUT_PREFIXES = [
  "tools/bazel/",
];

const GENERATED_PACKAGE_SCRIPT_PREFIXES = [
  "rust:crate:clippy:",
  "rust:crate:test:",
  "rust:crate:nextest:",
  "rust:ctx-http:test:",
];

const GENERATED_PACKAGE_SCRIPT_NAMES = new Set([
  "rust:package-scripts:sync",
  "rust:package-scripts:check",
]);

// These crates remain in the Cargo workspace for manual/local use, but the
// default CI/release gate surface should not auto-generate tasks for them.
const MANUAL_ONLY_RUST_CRATES = new Set(["ctx-worker-gateway"]);

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

function isGateManagedCrate(crateName) {
  return !MANUAL_ONLY_RUST_CRATES.has(crateName);
}

function filterGateManagedCrateNames(crateNames) {
  return [...new Set(crateNames)].filter((crateName) => isGateManagedCrate(crateName)).sort();
}

function getGateManagedCrates(graph) {
  return graph.crates.filter((crate) => isGateManagedCrate(crate.crateName));
}

function normalizePathForMatch(value) {
  return String(value).replace(/\\/g, "/").replace(/\/+$/, "");
}

function normalizeRepoRelativePath(value) {
  return normalizePathForMatch(value).replace(/^\.\//u, "").replace(/^\/+/u, "");
}

function isRustWorkspaceLevelInput(changedFile) {
  const normalized = normalizeRepoRelativePath(changedFile);
  if (!normalized) {
    return false;
  }
  if (REPO_RUST_BUILD_GRAPH_INPUTS.includes(normalized)) {
    return true;
  }
  if (REPO_RUST_BUILD_GRAPH_INPUT_PREFIXES.some((prefix) => normalized.startsWith(prefix))) {
    return true;
  }
  const coreRelative = normalized.replace(/^core\//u, "");
  return ROOT_RUST_INPUTS.includes(coreRelative);
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
  return getInputGlobsForCrates(graph, closure);
}

function getDependencyInputGlobsWithoutSelf(graph, crateName) {
  const closure = expandDependencies(graph, [crateName]).filter((name) => name !== crateName);
  return getInputGlobsForCrates(graph, closure);
}

function getInputGlobsForCrates(graph, crateNames) {
  const globs = [];
  for (const name of [...new Set(crateNames)].sort()) {
    const crate = graph.cratesByName.get(name);
    if (!crate) {
      throw new Error(`unknown crate in input-glob request: ${name}`);
    }
    globs.push(`${crate.relDir}/**`);
  }
  return [...new Set([...ROOT_RUST_INPUTS, ...globs])];
}

function getRustCrateClippyScriptName(crateName) {
  return `rust:crate:clippy:${crateName}`;
}

function getRustCrateTestScriptName(crateName) {
  return `rust:crate:test:${crateName}`;
}

function getRustCrateNextestScriptName(crateName) {
  return `rust:crate:nextest:${crateName}`;
}

function getCtxHttpSuiteInputGlobs(graph, suiteName) {
  const dependencyInputs = getDependencyInputGlobsWithoutSelf(graph, "ctx-http");
  const suiteScriptInputs = [...ROOT_RUST_INPUTS, ...CTX_HTTP_SUITE_SCRIPT_INPUTS];
  const baseInputs = [...dependencyInputs, ...suiteScriptInputs];
  if (suiteName === "all") {
    return [...new Set([...baseInputs, "crates/ctx-http/src/**", "crates/ctx-http/tests/**"])];
  }
  if (suiteName === "base") {
    return [...new Set([...baseInputs, "crates/ctx-http/src/**"])];
  }

  const { getCtxHttpSuiteByName } = require("./ctx_http_suites.cjs");
  const suite = getCtxHttpSuiteByName(suiteName);
  if (!suite) {
    throw new Error(`unknown ctx-http suite: ${suiteName}`);
  }
  const dependencyCrates = suite.dependencyCrates || [];
  const suiteDependencyInputs = getInputGlobsForCrates(graph, dependencyCrates).filter(
    (input) => !ROOT_RUST_INPUTS.includes(input),
  );
  return [
    ...new Set([
      ...suiteScriptInputs,
      ...CTX_HTTP_SHARED_SOURCE_GLOBS,
      ...(suite.sourceGlobs || []),
      "crates/ctx-http/tests/common/**",
      ...suite.testFiles.map((testFile) => `crates/ctx-http/tests/${testFile}.rs`),
      ...suiteDependencyInputs,
    ]),
  ];
}

function buildGeneratedPackageScripts(graph) {
  const scripts = {
    "rust:package-scripts:sync": "node scripts/sync_rust_package_scripts.cjs",
    "rust:package-scripts:check": "node scripts/sync_rust_package_scripts.cjs --check",
  };
  for (const crate of getGateManagedCrates(graph)) {
    scripts[getRustCrateClippyScriptName(crate.crateName)] =
      `node scripts/rust_crate_task.cjs --crate ${crate.crateName} --task clippy`;
    if (crate.crateName === "ctx-http") {
      scripts[getRustCrateTestScriptName(crate.crateName)] =
        "node scripts/ctx_http_suite_task.cjs --suite all";
    } else {
      scripts[getRustCrateTestScriptName(crate.crateName)] =
        `node scripts/rust_crate_task.cjs --crate ${crate.crateName} --task test`;
    }
    scripts[getRustCrateNextestScriptName(crate.crateName)] =
      `node scripts/rust_crate_task.cjs --crate ${crate.crateName} --task nextest`;
  }
  for (const suiteName of getCtxHttpSuiteNames({ includeAll: true })) {
    scripts[getCtxHttpSuiteTaskName(suiteName)] =
      `node scripts/ctx_http_suite_task.cjs --suite ${suiteName}`;
  }
  return scripts;
}

function getPackageScriptNamesForCrates(crateNames, taskKinds) {
  const scriptNames = [];
  for (const crateName of [...crateNames].sort()) {
    for (const taskKind of taskKinds) {
      if (taskKind === "clippy") {
        scriptNames.push(getRustCrateClippyScriptName(crateName));
      } else if (taskKind === "test") {
        if (crateName === "ctx-http") {
          const suiteTaskNames = getCtxHttpSuiteNamesForAllTarget()
            .map((suiteName) => getCtxHttpSuiteTaskName(suiteName));
          scriptNames.push(...suiteTaskNames);
        } else {
          scriptNames.push(getRustCrateTestScriptName(crateName));
        }
      } else if (taskKind === "nextest") {
        scriptNames.push(getRustCrateNextestScriptName(crateName));
      } else {
        throw new Error(`unknown Rust task kind: ${taskKind}`);
      }
    }
  }
  return scriptNames;
}

function getPackageTaskCommandsForCrates(graph, crateNames, taskKinds) {
  const scripts = buildGeneratedPackageScripts(graph);
  return getPackageScriptNamesForCrates(crateNames, taskKinds).map((scriptName) => {
    const command = scripts[scriptName];
    if (!command) {
      throw new Error(`missing generated package script: ${scriptName}`);
    }
    return {
      command,
      name: scriptName,
    };
  });
}

module.exports = {
  GENERATED_PACKAGE_SCRIPT_NAMES,
  GENERATED_PACKAGE_SCRIPT_PREFIXES,
  MANUAL_ONLY_RUST_CRATES,
  REPO_RUST_BUILD_GRAPH_INPUT_PREFIXES,
  REPO_RUST_BUILD_GRAPH_INPUTS,
  ROOT_RUST_INPUTS,
  buildGeneratedPackageScripts,
  buildWorkspaceGraph,
  collectChangedCrates,
  expandDependencies,
  expandReverseDependencies,
  filterGateManagedCrateNames,
  getCtxHttpSuiteTaskName,
  getClosureInputGlobs,
  getCrateByChangedPath,
  getGateManagedCrates,
  getPackageScriptNamesForCrates,
  getPackageTaskCommandsForCrates,
  getRustCrateClippyScriptName,
  getRustCrateNextestScriptName,
  getRustCrateTestScriptName,
  isRustWorkspaceLevelInput,
  normalizePathForMatch,
  runCargoMetadata,
};
