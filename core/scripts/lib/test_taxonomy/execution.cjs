const childProcess = require("node:child_process");
const path = require("node:path");

const { buildTaxonomyRegistry } = require("./registry.cjs");
const { getFamiliesById } = require("./families.cjs");
const { getProfiles, profileMatchesEntry } = require("./profiles.cjs");
const {
  ROOT_RUST_INPUTS,
  buildWorkspaceGraph,
  collectChangedCrates,
} = require("../rust_workspace_graph.cjs");

const coreRoot = path.resolve(__dirname, "..", "..", "..");
const repoRoot = path.resolve(coreRoot, "..");

function normalizeRepoRelativePath(value) {
  return String(value || "")
    .trim()
    .replace(/\\/gu, "/")
    .replace(/^\.\//u, "")
    .replace(/^\/+/u, "");
}

function shellQuote(value) {
  const stringValue = String(value);
  if (/^[A-Za-z0-9_./:=,@+-]+$/u.test(stringValue)) {
    return stringValue;
  }
  return `'${stringValue.replace(/'/gu, `'\"'\"'`)}'`;
}

function shellJoin(command, args = []) {
  return [command, ...args].map(shellQuote).join(" ");
}

function getProfileById(profileId) {
  const profiles = getProfiles(getFamiliesById());
  const profile = profiles.find((candidate) => candidate.id === profileId);
  if (!profile) {
    throw new Error(`unknown taxonomy profile: ${profileId}`);
  }
  return profile;
}

function matchesGlob(filePath, glob) {
  const normalizedPath = normalizeRepoRelativePath(filePath);
  const normalizedGlob = normalizeRepoRelativePath(glob);
  if (!normalizedGlob) {
    return false;
  }
  if (typeof path.matchesGlob === "function") {
    return path.matchesGlob(normalizedPath, normalizedGlob);
  }

  const regex = new RegExp(`^${normalizedGlob
    .replace(/[|\\{}()[\]^$+?.]/gu, "\\$&")
    .replace(/\*\*/gu, ":::DOUBLE_STAR:::")
    .replace(/\*/gu, "[^/]*")
    .replace(/:::DOUBLE_STAR:::/gu, ".*")}$`);
  return regex.test(normalizedPath);
}

function entryMatchesChangedFiles(entry, changedFiles) {
  const normalizedChangedFiles = [...new Set(changedFiles.map(normalizeRepoRelativePath).filter(Boolean))];
  if (normalizedChangedFiles.length === 0) {
    return false;
  }
  for (const changedFile of normalizedChangedFiles) {
    for (const glob of entry.sourceGlobs || []) {
      if (matchesGlob(changedFile, glob)) {
        return true;
      }
    }
  }
  return false;
}

function isAlwaysOnEntry(entry, profileId) {
  return Array.isArray(entry.alwaysOnProfiles) && entry.alwaysOnProfiles.includes(profileId);
}

function resolveWebSuiteScriptName(suite) {
  switch (suite) {
    case "premerge_required":
      return "test:e2e:premerge";
    case "release_required":
      return "test:e2e:release";
    case "cross_platform":
      return "test:e2e:cross-platform";
    case "visual":
      return "test:e2e:visual";
    case "soak":
      return "test:e2e:soak";
    case "load":
      return "test:e2e:load";
    default:
      throw new Error(`unsupported web e2e suite for command mapping: ${suite}`);
  }
}

function buildCommandForEntry(entry) {
  if (entry.entrypointType === "core-package-script") {
    return shellJoin("pnpm", [entry.entrypoint]);
  }
  if (entry.entrypointType === "file") {
    return shellJoin("node", ["--test", path.relative(coreRoot, path.join(repoRoot, entry.entrypoint)).replace(/\\/gu, "/")]);
  }
  if (entry.entrypointType === "ctx-http-suite") {
    return shellJoin("node", ["scripts/ctx_http_suite_task.cjs", "--suite", entry.entrypoint]);
  }
  if (entry.entrypointType === "provider-matrix-lane") {
    const lane = String(entry.entrypoint).split("#")[1];
    if (!lane) {
      throw new Error(`missing provider matrix lane on ${entry.id}`);
    }
    return shellJoin("pnpm", [`verify:desktop:provider-auth-matrix:${lane}`]);
  }
  if (entry.entrypointType === "web-e2e-spec") {
    return shellJoin("pnpm", [resolveWebSuiteScriptName(entry.suite)]);
  }
  throw new Error(`unsupported entrypoint type for command mapping: ${entry.entrypointType}`);
}

function getCompatibilityFallbackCommands({ changedFiles }) {
  const commands = [];
  const changed = [...new Set(changedFiles.map(normalizeRepoRelativePath).filter(Boolean))];
  const graph = buildWorkspaceGraph(coreRoot);
  const workspaceLevelRustChange = changed.some((changedFile) => {
    const normalized = changedFile.replace(/^core\//u, "");
    return ROOT_RUST_INPUTS.includes(normalized);
  });
  const changedCrates = collectChangedCrates(graph, changed);
  const nonCtxHttpCrates = changedCrates.filter((crateName) => crateName !== "ctx-http");

  if (workspaceLevelRustChange || nonCtxHttpCrates.length > 0) {
    commands.push(shellJoin("pnpm", ["rust:turbo:check"]));
    const rustArgs = [
      "exec",
      "node",
      "scripts/run_rust_gate.cjs",
      "--mode",
      "workspace",
      "--include-reverse-deps",
      "--clippy",
      "--test-strategy",
      "mixed",
    ];
    for (const changedFile of changed) {
      if (
        workspaceLevelRustChange ||
        changedFile.startsWith("core/crates/") ||
        changedFile.startsWith("core/tools/") ||
        changedFile === "core/Cargo.toml" ||
        changedFile === "core/Cargo.lock"
      ) {
        rustArgs.push("--changed-file", changedFile);
      }
    }
    commands.push(shellJoin("pnpm", rustArgs));
  }

  return commands;
}

function commandPriority(command) {
  if (command === "pnpm bazel:web:test") {
    return 100;
  }
  if (command === "pnpm verify:e2e") {
    return 200;
  }
  return 1000;
}

function dedupeCommands(commands) {
  return [...new Set(commands.filter(Boolean))]
    .map((command, index) => ({
      command,
      index,
      priority: commandPriority(command),
    }))
    .sort((left, right) => left.priority - right.priority || left.index - right.index)
    .map((entry) => entry.command);
}

function buildExecutionPlan({ profileId, changedFiles = [], touchedOnly = false }) {
  const registry = buildTaxonomyRegistry();
  const profile = getProfileById(profileId);
  const matchingEntries = registry.filter((entry) => profileMatchesEntry(profile, entry));

  let selectedEntries = matchingEntries;
  if (touchedOnly) {
    selectedEntries = matchingEntries.filter((entry) =>
      isAlwaysOnEntry(entry, profileId) || entryMatchesChangedFiles(entry, changedFiles),
    );
  }

  const entryCommands = selectedEntries.map(buildCommandForEntry);
  const compatibilityCommands = touchedOnly
    ? getCompatibilityFallbackCommands({ changedFiles })
    : [];
  const commands = dedupeCommands([...entryCommands, ...compatibilityCommands]);

  return {
    profile,
    selectedEntries,
    commands,
  };
}

function resolveChangedFilesFromGit(baseRef) {
  const args = ["diff", "--name-only"];
  if (baseRef) {
    args.push(`${baseRef}...HEAD`);
  } else {
    args.push("HEAD");
  }
  const output = childProcess.execFileSync("git", args, {
    cwd: repoRoot,
    encoding: "utf8",
  });
  return output.split(/\r?\n/u).map(normalizeRepoRelativePath).filter(Boolean);
}

module.exports = {
  buildExecutionPlan,
  buildCommandForEntry,
  entryMatchesChangedFiles,
  getCompatibilityFallbackCommands,
  normalizeRepoRelativePath,
  resolveChangedFilesFromGit,
  shellJoin,
};
