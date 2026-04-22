const fs = require("node:fs");
const path = require("node:path");

const {
  buildWorkspaceGraph,
  normalizePathForMatch,
  runCargoMetadata,
} = require("./rust_workspace_graph.cjs");

function sortUnique(values) {
  return [...new Set(values)].filter(Boolean).sort();
}

function assertConfigShape(config) {
  if (!config || config.version !== 1) {
    throw new Error("rust_bazel_deps config must have version 1");
  }
  for (const [field, value] of [
    ["generatedDepConsumerCrates", config.generatedDepConsumerCrates],
    ["manualCrates", config.manualCrates],
    ["procMacroDeps", config.procMacroDeps],
  ]) {
    if (!value || typeof value !== "object") {
      throw new Error(`rust_bazel_deps config field ${field} must be an object`);
    }
  }
  if (!Array.isArray(config.generatedDepConsumerCrates)) {
    throw new Error("rust_bazel_deps config field generatedDepConsumerCrates must be an array");
  }
  for (const [crateName, entry] of Object.entries(config.manualCrates)) {
    requireRationale(`manualCrates.${crateName}`, entry);
  }
  for (const [depName, entry] of Object.entries(config.procMacroDeps)) {
    requireRationale(`procMacroDeps.${depName}`, entry);
  }
}

function requireRationale(name, entry) {
  if (!entry || typeof entry !== "object") {
    throw new Error(`${name} must be an object with owner and rationale`);
  }
  if (!String(entry.owner || "").trim()) {
    throw new Error(`${name} must include owner`);
  }
  if (!String(entry.rationale || "").trim()) {
    throw new Error(`${name} must include rationale`);
  }
}

function parseModuleCrateSpecs(moduleBazelText) {
  const specs = new Set();
  const pattern = /package\s*=\s*"([^"]+)"/g;
  let match = pattern.exec(moduleBazelText);
  while (match) {
    specs.add(match[1]);
    match = pattern.exec(moduleBazelText);
  }
  return specs;
}

function packageRelDir(coreRoot, manifestPath) {
  return normalizePathForMatch(path.relative(coreRoot, path.dirname(manifestPath)));
}

function buildPackageIndexes(coreRoot, metadata) {
  const workspaceMemberIds = new Set(metadata.workspace_members);
  const packages = metadata.packages
    .filter((pkg) => workspaceMemberIds.has(pkg.id))
    .sort((a, b) => a.name.localeCompare(b.name));
  const byName = new Map();
  const byDir = new Map();
  for (const pkg of packages) {
    const relDir = packageRelDir(coreRoot, pkg.manifest_path);
    const indexed = {
      ...pkg,
      relDir,
    };
    byName.set(pkg.name, indexed);
    byDir.set(path.resolve(path.dirname(pkg.manifest_path)), indexed);
  }
  return {
    byDir,
    byName,
    packages,
  };
}

function defaultWorkspaceDepLabel(depPkg) {
  return `//core/${depPkg.relDir}:lib`;
}

function hasBazelTarget(buildText, targetName) {
  const escapedName = targetName.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  return new RegExp(`name\\s*=\\s*"${escapedName}"`).test(buildText);
}

function validateWorkspaceDepTarget({ buildPath, depName, fileExists, readFile }) {
  if (!fileExists(buildPath)) {
    throw new Error(`path dependency '${depName}' has no BUILD.bazel at ${buildPath}`);
  }
  const buildText = readFile(buildPath, "utf8");
  if (!hasBazelTarget(buildText, "lib")) {
    throw new Error(`path dependency '${depName}' must expose a Bazel target named 'lib' in ${buildPath}`);
  }
}

function labelForDependency({ coreRoot, dep, fileExists, packageIndexes, moduleCrateSpecs, includedExternalDeps, readFile }) {
  if (dep.rename) {
    throw new Error(
      `dependency '${dep.rename}' renames package '${dep.name}', but generated Bazel deps do not support renamed Cargo dependencies yet`,
    );
  }

  if (dep.path) {
    const depPkg = packageIndexes.byDir.get(path.resolve(dep.path));
    if (!depPkg) {
      throw new Error(`path dependency ${dep.name} points outside the core Cargo workspace: ${dep.path}`);
    }
    const buildPath = path.join(coreRoot, depPkg.relDir, "BUILD.bazel");
    validateWorkspaceDepTarget({ buildPath, depName: dep.name, fileExists, readFile });
    return defaultWorkspaceDepLabel(depPkg);
  }

  if (!moduleCrateSpecs.has(dep.name)) {
    throw new Error(
      `Cargo dependency '${dep.name}' is not declared in MODULE.bazel crate.spec(...). `
      + "Add a crate.spec entry or mark the owning crate manual before regenerating Bazel deps.",
    );
  }
  includedExternalDeps.add(dep.name);
  return `@crates//:${dep.name}`;
}

function shouldIncludeDependency(dep) {
  if (dep.optional) {
    return false;
  }
  if (dep.target) {
    return false;
  }
  return true;
}

function emptyEntry() {
  return {
    build_deps: [],
    deps: [],
    dev_deps: [],
    dev_proc_macro_deps: [],
    proc_macro_deps: [],
  };
}

function addLabel(entry, field, label) {
  entry[field].push(label);
}

function fieldForDependency(dep, config) {
  const isProcMacro = Boolean(config.procMacroDeps[dep.name]);
  if (dep.kind === "build") {
    return "build_deps";
  }
  if (dep.kind === "dev") {
    return isProcMacro ? "dev_proc_macro_deps" : "dev_deps";
  }
  return isProcMacro ? "proc_macro_deps" : "deps";
}

function buildRustBazelDeps({
  config,
  coreRoot,
  fileExists = fs.existsSync,
  metadata = runCargoMetadata(coreRoot),
  moduleBazelText,
  readFile = fs.readFileSync,
  repoRoot = path.resolve(coreRoot, ".."),
} = {}) {
  assertConfigShape(config);
  const moduleText = moduleBazelText ?? fs.readFileSync(path.join(repoRoot, "MODULE.bazel"), "utf8");
  const moduleCrateSpecs = parseModuleCrateSpecs(moduleText);
  const packageIndexes = buildPackageIndexes(coreRoot, metadata);
  const entries = {};
  const skippedCrates = {};
  const includedExternalDeps = new Set();
  const generatedCrates = new Set(config.generatedDepConsumerCrates);

  for (const crateName of generatedCrates) {
    if (!packageIndexes.byName.has(crateName)) {
      throw new Error(`generatedDepConsumerCrates includes unknown workspace crate '${crateName}'`);
    }
    if (config.manualCrates[crateName]) {
      throw new Error(`generatedDepConsumerCrates includes manual crate '${crateName}'`);
    }
  }

  for (const pkg of packageIndexes.packages) {
    if (config.manualCrates[pkg.name]) {
      skippedCrates[pkg.name] = config.manualCrates[pkg.name].rationale;
      continue;
    }
    if (!generatedCrates.has(pkg.name)) {
      continue;
    }

    const indexedPkg = packageIndexes.byName.get(pkg.name);
    const buildPath = path.join(coreRoot, indexedPkg.relDir, "BUILD.bazel");
    if (!fileExists(buildPath)) {
      throw new Error(`workspace crate '${pkg.name}' is not manual and has no BUILD.bazel at ${buildPath}`);
    }

    const entry = emptyEntry();
    for (const dep of pkg.dependencies) {
      if (!shouldIncludeDependency(dep)) {
        continue;
      }
      const label = labelForDependency({
        coreRoot,
        dep,
        fileExists,
        includedExternalDeps,
        moduleCrateSpecs,
        packageIndexes,
        readFile,
      });
      addLabel(entry, fieldForDependency(dep, config), label);
    }
    for (const field of Object.keys(entry)) {
      entry[field] = sortUnique(entry[field]);
    }
    entries[pkg.name] = entry;
  }

  return {
    entries,
    includedExternalDeps: sortUnique([...includedExternalDeps]),
    skippedCrates,
  };
}

function renderStringList(values, indent) {
  if (values.length === 0) {
    return "[]";
  }
  const pad = " ".repeat(indent);
  const itemPad = " ".repeat(indent + 4);
  return `[\n${values.map((value) => `${itemPad}"${value}",`).join("\n")}\n${pad}]`;
}

function renderRustBazelDepsStarlark(model) {
  const lines = [
    "# @generated by core/scripts/sync_rust_bazel_deps.cjs",
    "# Source of truth: core/Cargo.toml via cargo metadata.",
    "# Do not edit manually. Run `pnpm rust:bazel-deps:write` from core/.",
    "",
    "RUST_BAZEL_DEPS = {",
  ];
  for (const crateName of Object.keys(model.entries).sort()) {
    const entry = model.entries[crateName];
    lines.push(`    "${crateName}": struct(`);
    for (const field of ["build_deps", "deps", "dev_deps", "dev_proc_macro_deps", "proc_macro_deps"]) {
      lines.push(`        ${field} = ${renderStringList(entry[field], 8)},`);
    }
    lines.push("    ),");
  }
  lines.push("}");
  lines.push("");
  return lines.join("\n");
}

function buildGeneratedRustBazelDeps({
  config,
  coreRoot,
  fileExists,
  metadata,
  moduleBazelText,
  readFile,
  repoRoot,
} = {}) {
  return renderRustBazelDepsStarlark(buildRustBazelDeps({
    config,
    coreRoot,
    fileExists,
    metadata,
    moduleBazelText,
    readFile,
    repoRoot,
  }));
}

function compareGeneratedFile({ content, outPath, readFile = fs.readFileSync }) {
  let existing = "";
  try {
    existing = readFile(outPath, "utf8");
  } catch (error) {
    if (!error || error.code !== "ENOENT") {
      throw error;
    }
  }
  return existing === content;
}

function writeGeneratedFile({ content, mkdir = fs.mkdirSync, outPath, writeFile = fs.writeFileSync }) {
  mkdir(path.dirname(outPath), { recursive: true });
  writeFile(outPath, content);
}

function syncGeneratedRustBazelDeps({
  check = false,
  config,
  coreRoot,
  fileExists,
  mkdir,
  metadata,
  moduleBazelText,
  outPath,
  readFile,
  repoRoot,
  writeFile,
} = {}) {
  const content = buildGeneratedRustBazelDeps({
    config,
    coreRoot,
    fileExists,
    metadata,
    moduleBazelText,
    readFile,
    repoRoot,
  });
  const current = compareGeneratedFile({ content, outPath, readFile });
  if (check) {
    if (!current) {
      throw new Error(`generated Rust Bazel deps are stale: ${outPath}\nRun: pnpm rust:bazel-deps:write`);
    }
    return {
      content,
      current: true,
      wrote: false,
    };
  }
  if (!current) {
    writeGeneratedFile({ content, mkdir, outPath, writeFile });
  }
  return {
    content,
    current,
    wrote: !current,
  };
}

function buildRealWorkspaceModel({ config, coreRoot, repoRoot } = {}) {
  const metadata = runCargoMetadata(coreRoot);
  const graph = buildWorkspaceGraph(coreRoot, metadata);
  const model = buildRustBazelDeps({
    config,
    coreRoot,
    metadata,
    repoRoot,
  });
  return {
    graph,
    model,
  };
}

module.exports = {
  buildGeneratedRustBazelDeps,
  buildPackageIndexes,
  buildRealWorkspaceModel,
  buildRustBazelDeps,
  compareGeneratedFile,
  parseModuleCrateSpecs,
  renderRustBazelDepsStarlark,
  syncGeneratedRustBazelDeps,
  writeGeneratedFile,
};
