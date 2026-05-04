const path = require("node:path");

const { buildTaxonomyRegistry } = require("./registry.cjs");
const { getFamiliesById } = require("./families.cjs");
const { getProfiles, profileMatchesEntry } = require("./profiles.cjs");
const {
  buildCtxHttpSuiteTaskArgs,
  expandCtxHttpSuiteForPlanner,
  getCtxHttpSuiteCheckinFanoutTargets,
} = require("../ctx_http_suites.cjs");
const {
  resolveMergeBaseFiles,
  resolveWorkingTreeFiles,
} = require("../verification_git_changes.cjs");
const {
  buildWorkspaceGraph,
  collectChangedCrates,
  expandReverseDependencies,
  filterGateManagedCrateNames,
  isRustWorkspaceLevelInput,
} = require("../rust_workspace_graph.cjs");

const coreRoot = path.resolve(__dirname, "..", "..", "..");
const repoRoot = path.resolve(coreRoot, "..");
const workspaceGraph = buildWorkspaceGraph(coreRoot);
const CHECKIN_BUILDKITE_RUST_GATE_CHUNK_SIZE = 12;
const CHECKIN_RUST_GATE_TEST_EXCLUDED_RESOLVED_CRATES = new Set(["ctx-http"]);
const CHECKIN_RUST_GATE_CRATE_WEIGHTS = Object.freeze({
  "ctx-avf-linux-runtime": 3,
  "ctx-client": 2,
  "ctx-execution-runtime": 2,
  "ctx-harness-runtime": 2,
  "ctx-harness-setup": 2,
  "ctx-linux-sandbox-runtime": 3,
  "ctx-managed-installs": 5,
  "ctx-mcp": 5,
  "ctx-provider-matrix": 2,
  "ctx-provider-runtime": 2,
  "ctx-providers": 3,
  "ctx-store": 4,
  "ctx-workspace-active-snapshot": 2,
  "ctx-workspace-container": 2,
  "ctx-workspace-runtime": 3,
});
const CHECKIN_BUILDKITE_EXECUTION_OPTIONS = Object.freeze({
  coalesceCtxHttpSuites: false,
  fanoutCtxHttpSuiteTargets: true,
  rustGateChunkSize: CHECKIN_BUILDKITE_RUST_GATE_CHUNK_SIZE,
});

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

function buildChangedContext(changedFiles) {
  const normalizedChangedFiles = [...new Set(changedFiles.map(normalizeRepoRelativePath).filter(Boolean))];
  const changedCrates = collectChangedCrates(workspaceGraph, normalizedChangedFiles);
  const workspaceLevelRustChange = normalizedChangedFiles.some(isRustWorkspaceLevelInput);
  return {
    normalizedChangedFiles,
    changedCrates,
    workspaceLevelRustChange,
  };
}

function entryMatchesChangedFiles(entry, changedFilesOrContext, { selectionMode = "touched" } = {}) {
  const changedContext = Array.isArray(changedFilesOrContext)
    ? buildChangedContext(changedFilesOrContext)
    : changedFilesOrContext;
  if (!changedContext || changedContext.normalizedChangedFiles.length === 0) {
    return false;
  }
  for (const changedFile of changedContext.normalizedChangedFiles) {
    if ((entry.excludeGlobs || []).some((glob) => matchesGlob(changedFile, glob))) {
      continue;
    }
    for (const glob of entry.sourceGlobs || []) {
      if (matchesGlob(changedFile, glob)) {
        return true;
      }
    }
    if (selectionMode === "affected") {
      for (const glob of entry.affectedGlobs || []) {
        if (matchesGlob(changedFile, glob)) {
          return true;
        }
      }
    }
  }
  if (entry.entrypointType === "rust-crate-gate" && changedContext.workspaceLevelRustChange) {
    return true;
  }
  const supportsCrateMatching = selectionMode === "touched"
    ? entry.entrypointType === "rust-crate-gate" || entry.id === "build-graph.rust-package-scripts-check"
    : true;
  if (
    supportsCrateMatching
    && (entry.dependencyCrates || []).some((crateName) => changedContext.changedCrates.includes(crateName))
  ) {
    return true;
  }
  return false;
}

function isAlwaysOnEntry(entry, profileId) {
  return Array.isArray(entry.alwaysOnProfiles) && entry.alwaysOnProfiles.includes(profileId);
}

function profileMatchesTouchedOnlyEscalation(profile, entry) {
  if (profile.id !== "agent-default") {
    return false;
  }
  if (entry.id === "web-workbench.web-premerge-required") {
    return true;
  }
  return entry.entrypointType === "web-e2e-spec";
}

function profileMatchesAffectedExpansion(profile, entry) {
  if (profile.id === "agent-default") {
    return entry.id === "web-workbench.web-premerge-required";
  }
  return false;
}

function buildCommandForEntry(entry) {
  if (entry.entrypointType === "web-e2e-spec") {
    const bazelLabels = entry.executionTargets?.webE2E?.bazelLabels ?? [];
    if (bazelLabels.length === 0) {
      throw new Error(`web E2E entry is missing Bazel labels: ${entry.id}`);
    }
    return shellJoin("node", ["scripts/run_bazel_pilot.cjs", "test", ...bazelLabels]);
  }
  if (entry.entrypointType === "core-package-script") {
    return shellJoin("pnpm", [entry.entrypoint]);
  }
  if (entry.entrypointType === "repo-shell-script") {
    return shellJoin("bash", [path.relative(coreRoot, path.join(repoRoot, entry.entrypoint)).replace(/\\/gu, "/")]);
  }
  if (entry.entrypointType === "file") {
    return shellJoin("node", ["--test", path.relative(coreRoot, path.join(repoRoot, entry.entrypoint)).replace(/\\/gu, "/")]);
  }
  if (entry.entrypointType === "ctx-http-suite") {
    return shellJoin("node", [
      "scripts/ctx_http_suite_task.cjs",
      ...buildCtxHttpSuiteTaskArgs(expandCtxHttpSuiteForPlanner(entry.entrypoint)),
    ]);
  }
  if (entry.entrypointType === "provider-matrix-lane") {
    const lane = String(entry.entrypoint).split("#")[1];
    if (!lane) {
      throw new Error(`missing provider matrix lane on ${entry.id}`);
    }
    return shellJoin("pnpm", [`verify:desktop:provider-auth-matrix:${lane}`]);
  }
  throw new Error(`unsupported entrypoint type for command mapping: ${entry.entrypointType}`);
}

function buildCtxHttpSuiteFanoutCommands(suiteName) {
  return expandCtxHttpSuiteForPlanner(suiteName)
    .flatMap((entry) => getCtxHttpSuiteCheckinFanoutTargets(entry))
    .map((target) => shellJoin("node", ["scripts/run_bazel_pilot.cjs", "test", target]));
}

function buildRustGateCommandForCrates(crateNames, {
  includeReverseDeps = true,
  resolvedCrates = false,
  skipTests = false,
} = {}) {
  const args = [
    "exec",
    "node",
    "scripts/run_rust_gate.cjs",
    "--mode",
    "workspace",
  ];
  if (includeReverseDeps) {
    args.push("--include-reverse-deps");
  }
  args.push("--clippy", "--test-strategy", "mixed");
  if (resolvedCrates) {
    args.push("--resolved-crates");
  }
  if (skipTests) {
    args.push("--skip-tests");
  }

  for (const crateName of crateNames) {
    args.push("--crate", crateName);
  }

  return shellJoin("pnpm", args);
}

function buildRustGateCommand({ rustGateEntries, changedContext, selectionMode }) {
  const args = [
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

  if (selectionMode === "touched") {
    for (const changedFile of changedContext.normalizedChangedFiles) {
      if (
        changedContext.workspaceLevelRustChange ||
        changedFile.startsWith("core/crates/") ||
        changedFile.startsWith("core/tools/") ||
        changedFile === "core/Cargo.toml" ||
        changedFile === "core/Cargo.lock"
      ) {
        args.push("--changed-file", changedFile);
      }
    }
  } else {
    return buildRustGateCommandForCrates([...new Set(rustGateEntries.map((entry) => entry.entrypoint))].sort());
  }

  return shellJoin("pnpm", args);
}

function resolveRustGateCrateWeight(crateName, weightMap = CHECKIN_RUST_GATE_CRATE_WEIGHTS) {
  const weight = Number(weightMap[String(crateName || "")]);
  return Number.isFinite(weight) && weight > 0 ? weight : 1;
}

function buildWeightedChunks(values, {
  chunkSize = 0,
  weightForValue = resolveRustGateCrateWeight,
} = {}) {
  if (values.length === 0) {
    return [];
  }
  if (!Number.isInteger(chunkSize) || chunkSize <= 0 || values.length <= chunkSize) {
    return [values];
  }

  const chunkCount = Math.ceil(values.length / chunkSize);
  const chunks = Array.from({ length: chunkCount }, (_, index) => ({
    index,
    items: [],
    weight: 0,
  }));
  const weightedValues = [...values].sort((left, right) => {
    const weightDelta = weightForValue(right) - weightForValue(left);
    return weightDelta || left.localeCompare(right);
  });

  for (const value of weightedValues) {
    const valueWeight = weightForValue(value);
    const targetChunk = chunks
      .filter((chunk) => chunk.items.length < chunkSize)
      .sort((left, right) =>
        left.weight - right.weight
        || left.items.length - right.items.length
        || left.index - right.index,
      )[0];
    targetChunk.items.push(value);
    targetChunk.weight += valueWeight;
  }

  return chunks
    .map((chunk) => chunk.items.sort())
    .filter((items) => items.length > 0)
    .sort((left, right) => left[0].localeCompare(right[0]));
}

function buildResolvedRustGateCrates(rustGateEntries) {
  const inputCrates = [...new Set(rustGateEntries.map((entry) => entry.entrypoint))].sort();
  return filterGateManagedCrateNames(expandReverseDependencies(workspaceGraph, inputCrates));
}

function buildResolvedRustGateChunks(rustGateEntries, {
  chunkSize = 0,
  testExcludedCrates = CHECKIN_RUST_GATE_TEST_EXCLUDED_RESOLVED_CRATES,
} = {}) {
  const testCrates = buildResolvedRustGateCrates(rustGateEntries)
    .filter((crateName) => !testExcludedCrates.has(crateName));
  return buildWeightedChunks(testCrates, { chunkSize });
}

function buildResolvedRustGateClippyOnlyCrates(rustGateEntries, {
  clippyOnlyCrates = CHECKIN_RUST_GATE_TEST_EXCLUDED_RESOLVED_CRATES,
} = {}) {
  return buildResolvedRustGateCrates(rustGateEntries)
    .filter((crateName) => clippyOnlyCrates.has(crateName));
}

function buildCommandsForEntries({
  selectedEntries,
  changedContext,
  selectionMode,
  coalesceCtxHttpSuites = true,
  fanoutCtxHttpSuiteTargets = false,
  rustGateChunkSize = 0,
}) {
  const commands = [];
  const ctxHttpSuites = [];
  const rustGateEntries = [];
  const flushCtxHttpSuites = () => {
    if (ctxHttpSuites.length === 0) {
      return;
    }
    commands.push(shellJoin("node", ["scripts/ctx_http_suite_task.cjs", ...buildCtxHttpSuiteTaskArgs(ctxHttpSuites)]));
    ctxHttpSuites.length = 0;
  };

  for (const entry of selectedEntries) {
    if (entry.entrypointType === "rust-crate-gate") {
      flushCtxHttpSuites();
      rustGateEntries.push(entry);
      continue;
    }
    if (entry.entrypointType === "ctx-http-suite" && coalesceCtxHttpSuites) {
      ctxHttpSuites.push(...expandCtxHttpSuiteForPlanner(entry.entrypoint));
      continue;
    }
    flushCtxHttpSuites();
    if (entry.entrypointType === "ctx-http-suite" && fanoutCtxHttpSuiteTargets) {
      commands.push(...buildCtxHttpSuiteFanoutCommands(entry.entrypoint));
      continue;
    }
    commands.push(buildCommandForEntry(entry));
  }
  flushCtxHttpSuites();

  if (rustGateEntries.length > 0) {
    if (selectionMode === "all" && rustGateChunkSize > 0) {
      for (const crateChunk of buildResolvedRustGateChunks(rustGateEntries, { chunkSize: rustGateChunkSize })) {
        commands.push(buildRustGateCommandForCrates(crateChunk, {
          includeReverseDeps: false,
          resolvedCrates: true,
        }));
      }
      const clippyOnlyCrates = buildResolvedRustGateClippyOnlyCrates(rustGateEntries);
      if (clippyOnlyCrates.length > 0) {
        commands.push(buildRustGateCommandForCrates(clippyOnlyCrates, {
          includeReverseDeps: false,
          resolvedCrates: true,
          skipTests: true,
        }));
      }
    } else {
      commands.push(buildRustGateCommand({
        rustGateEntries,
        changedContext,
        selectionMode,
      }));
    }
  }

  return commands;
}

function normalizeSelectionMode({ selectionMode = "", touchedOnly = false } = {}) {
  const normalized = touchedOnly ? "touched" : String(selectionMode || "").trim().toLowerCase();
  if (!normalized) {
    return "all";
  }
  if (!["all", "touched", "affected"].includes(normalized)) {
    throw new Error(`unsupported selection mode: ${selectionMode}`);
  }
  return normalized;
}

function commandPriority(command) {
  if (
    command === "pnpm bazel:web:unit:supervisor-core"
    || command === "pnpm bazel:web:unit:thread-layout"
  ) {
    return 90;
  }
  if (
    command === "pnpm bazel:web:test"
    || command === "pnpm bazel:web:unit:non-pretext"
    || command === "pnpm bazel:web:unit:non-pretext:foundation"
    || command === "pnpm bazel:web:unit:non-pretext:foundation:shared"
    || command === "pnpm bazel:web:unit:non-pretext:foundation:state"
    || command === "pnpm bazel:web:unit:non-pretext:settings-setup"
    || command === "pnpm bazel:web:unit:non-pretext:workbench"
    || command === "pnpm bazel:web:unit:non-pretext:workbench:surface"
    || command === "pnpm bazel:web:unit:non-pretext:workbench:surface:app"
    || command === "pnpm bazel:web:unit:non-pretext:workbench:surface:session"
    || command === "pnpm bazel:web:unit:non-pretext:workbench:shell"
    || command === "pnpm bazel:web:unit:non-pretext:misc"
    || command === "pnpm bazel:web:pretext:measurement"
  ) {
    return 100;
  }
  if (command === "pnpm bazel:web:e2e:premerge") {
    return 200;
  }
  if (command.startsWith("node scripts/run_bazel_pilot.cjs test //core/apps/web/e2e:")) {
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

function orderSelectedEntries(entries, profile) {
  const explicitOrder = [
    ...(profile.selector?.forceIncludeEntryIds || []),
    ...(profile.selector?.includeEntryIds || []),
  ];
  if (explicitOrder.length === 0) {
    return entries;
  }
  const orderById = new Map(explicitOrder.map((id, index) => [id, index]));
  return [...entries]
    .map((entry, index) => ({
      entry,
      index,
      explicitIndex: orderById.has(entry.id) ? orderById.get(entry.id) : Number.POSITIVE_INFINITY,
    }))
    .sort((left, right) => left.explicitIndex - right.explicitIndex || left.index - right.index)
    .map(({ entry }) => entry);
}

function removeRedundantCtxHttpBaseEntry(entries) {
  const hasSpecificCtxHttpSuite = entries.some((entry) =>
    entry.entrypointType === "ctx-http-suite" && entry.entrypoint !== "base",
  );
  if (!hasSpecificCtxHttpSuite) {
    return entries;
  }
  return entries.filter((entry) =>
    !(entry.entrypointType === "ctx-http-suite" && entry.entrypoint === "base"),
  );
}

function buildExecutionPlan({
  profileId,
  changedFiles = [],
  touchedOnly = false,
  selectionMode = "",
  coalesceCtxHttpSuites = true,
  fanoutCtxHttpSuiteTargets = false,
  rustGateChunkSize = 0,
} = {}) {
  const registry = buildTaxonomyRegistry();
  const profile = getProfileById(profileId);
  const resolvedSelectionMode = normalizeSelectionMode({ selectionMode, touchedOnly });
  const matchingEntries = registry.filter((entry) =>
    profileMatchesEntry(profile, entry)
    || (resolvedSelectionMode === "touched" && profileMatchesTouchedOnlyEscalation(profile, entry))
    || (resolvedSelectionMode === "affected" && profileMatchesAffectedExpansion(profile, entry)),
  );
  const changedContext = buildChangedContext(changedFiles);

  let selectedEntries = matchingEntries;
  if (resolvedSelectionMode !== "all") {
    selectedEntries = matchingEntries.filter((entry) =>
      isAlwaysOnEntry(entry, profileId) || entryMatchesChangedFiles(entry, changedContext, {
        selectionMode: resolvedSelectionMode,
      }),
    );
    selectedEntries = removeRedundantCtxHttpBaseEntry(selectedEntries);
  }
  selectedEntries = orderSelectedEntries(selectedEntries, profile);

  const commands = dedupeCommands(buildCommandsForEntries({
    selectedEntries,
    changedContext,
    coalesceCtxHttpSuites,
    fanoutCtxHttpSuiteTargets,
    rustGateChunkSize,
    selectionMode: resolvedSelectionMode,
  }));

  return {
    profile,
    selectionMode: resolvedSelectionMode,
    selectedEntries,
    commands,
  };
}

function buildCheckinBuildkiteExecutionPlan({ profileId = "checkin" } = {}) {
  return buildExecutionPlan({
    changedFiles: [],
    profileId,
    ...CHECKIN_BUILDKITE_EXECUTION_OPTIONS,
  });
}

function buildExecutionPlanArtifact(plan, { changedFiles = [] } = {}) {
  return {
    changedFiles,
    commands: plan.commands,
    entries: plan.selectedEntries.map((entry) => entry.id),
    profile: plan.profile.id,
  };
}

function resolveChangedFilesFromGit(baseRef) {
  if (baseRef) {
    return resolveMergeBaseFiles(baseRef, { cwd: repoRoot }).changedFiles;
  }
  return resolveWorkingTreeFiles({ cwd: repoRoot });
}

module.exports = {
  CHECKIN_BUILDKITE_EXECUTION_OPTIONS,
  CHECKIN_BUILDKITE_RUST_GATE_CHUNK_SIZE,
  CHECKIN_RUST_GATE_CRATE_WEIGHTS,
  buildCtxHttpSuiteFanoutCommands,
  buildResolvedRustGateClippyOnlyCrates,
  buildResolvedRustGateChunks,
  buildCheckinBuildkiteExecutionPlan,
  buildExecutionPlan,
  buildExecutionPlanArtifact,
  buildCommandForEntry,
  buildChangedContext,
  buildWeightedChunks,
  entryMatchesChangedFiles,
  normalizeSelectionMode,
  normalizeRepoRelativePath,
  removeRedundantCtxHttpBaseEntry,
  resolveRustGateCrateWeight,
  resolveChangedFilesFromGit,
  shellJoin,
};
