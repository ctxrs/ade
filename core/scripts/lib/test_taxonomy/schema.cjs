const ENTRYPOINT_TYPES = [
  "file",
  "core-package-script",
  "ctx-http-suite",
  "provider-matrix-lane",
  "web-e2e-spec",
];

const SURFACES = [
  "contract",
  "compile",
  "unit",
  "integration",
  "system",
  "artifact",
  "promotion",
  "resilience",
  "performance",
  "adversarial",
];

const ORACLES = [
  "static-contract",
  "compiler",
  "direct-assertion",
  "golden-flow",
  "artifact-integrity",
  "live-publish",
  "budget",
  "fault-injection",
];

const WORLDS = [
  "hermetic",
  "simulated",
  "fake-provider",
  "local-packaged-artifact",
  "published-artifact",
  "live-provider",
  "external-service",
];

const COSTS = [
  "tiny",
  "fast",
  "medium",
  "slow",
  "soak",
];

const REQUIREMENTS = [
  "linux",
  "linux-arm64",
  "mac",
  "browser",
  "docker",
  "network",
  "single-mac",
  "buildbuddy-rbe",
  "long-running",
];

const STABILITIES = [
  "stable",
  "quarantined",
  "experimental",
];

const EXECUTIONS = [
  "bazel-rbe-preferred",
  "bazel-addressable",
  "script-local",
  "artifact-tail",
];

function assertAllowed(field, allowed, value, entryId) {
  if (!allowed.includes(value)) {
    throw new Error(`invalid ${field} '${value}' on ${entryId}; expected one of ${allowed.join(", ")}`);
  }
}

function assertString(field, value, entryId) {
  if (typeof value !== "string" || value.trim().length === 0) {
    throw new Error(`invalid ${field} on ${entryId}; expected non-empty string`);
  }
}

function normalizeStringArray(values) {
  return [...new Set((values || []).map((value) => String(value).trim()).filter(Boolean))].sort();
}

function validateEntry(entry, familiesById) {
  assertString("id", entry.id, "<entry>");
  assertString("title", entry.title, entry.id);
  assertString("family", entry.family, entry.id);
  if (!familiesById.has(entry.family)) {
    throw new Error(`unknown family '${entry.family}' on ${entry.id}`);
  }
  assertAllowed("entrypointType", ENTRYPOINT_TYPES, entry.entrypointType, entry.id);
  assertString("entrypoint", entry.entrypoint, entry.id);
  assertAllowed("surface", SURFACES, entry.surface, entry.id);
  assertAllowed("oracle", ORACLES, entry.oracle, entry.id);
  assertAllowed("world", WORLDS, entry.world, entry.id);
  assertAllowed("cost", COSTS, entry.cost, entry.id);
  assertAllowed("stability", STABILITIES, entry.stability, entry.id);
  assertAllowed("execution", EXECUTIONS, entry.execution, entry.id);
  assertString("owner", entry.owner, entry.id);

  entry.requirements = normalizeStringArray(entry.requirements);
  for (const requirement of entry.requirements) {
    assertAllowed("requirement", REQUIREMENTS, requirement, entry.id);
  }

  entry.sourceGlobs = normalizeStringArray(entry.sourceGlobs);
  entry.dependencyCrates = normalizeStringArray(entry.dependencyCrates);
  entry.notes = typeof entry.notes === "string" ? entry.notes.trim() : "";
  entry.exception = typeof entry.exception === "string" ? entry.exception.trim() : "";

  return entry;
}

function normalizeSelectorArray(values) {
  if (!Array.isArray(values) || values.length === 0) {
    return [];
  }
  return [...new Set(values.map((value) => String(value).trim()).filter(Boolean))].sort();
}

function validateProfile(profile, familiesById) {
  assertString("id", profile.id, "<profile>");
  assertString("title", profile.title, profile.id);
  assertString("purpose", profile.purpose, profile.id);
  assertString("remoteStrategy", profile.remoteStrategy, profile.id);
  assertString("currentExecution", profile.currentExecution, profile.id);
  profile.currentCommands = normalizeStringArray(profile.currentCommands);
  profile.pipelines = normalizeStringArray(profile.pipelines);
  profile.expansionRules = normalizeStringArray(profile.expansionRules);

  const selector = profile.selector || {};
  selector.families = normalizeSelectorArray(selector.families);
  selector.includeSurfaces = normalizeSelectorArray(selector.includeSurfaces);
  selector.includeWorlds = normalizeSelectorArray(selector.includeWorlds);
  selector.includeCosts = normalizeSelectorArray(selector.includeCosts);
  selector.includeStabilities = normalizeSelectorArray(selector.includeStabilities);
  selector.includeExecutions = normalizeSelectorArray(selector.includeExecutions);
  selector.excludeRequirements = normalizeSelectorArray(selector.excludeRequirements);
  selector.includeEntryIds = normalizeSelectorArray(selector.includeEntryIds);
  selector.forceIncludeEntryIds = normalizeSelectorArray(selector.forceIncludeEntryIds);
  selector.excludeEntryIds = normalizeSelectorArray(selector.excludeEntryIds);

  for (const family of selector.families) {
    if (!familiesById.has(family)) {
      throw new Error(`unknown selector family '${family}' on profile ${profile.id}`);
    }
  }
  for (const surface of selector.includeSurfaces) {
    assertAllowed("selector surface", SURFACES, surface, profile.id);
  }
  for (const world of selector.includeWorlds) {
    assertAllowed("selector world", WORLDS, world, profile.id);
  }
  for (const cost of selector.includeCosts) {
    assertAllowed("selector cost", COSTS, cost, profile.id);
  }
  for (const stability of selector.includeStabilities) {
    assertAllowed("selector stability", STABILITIES, stability, profile.id);
  }
  for (const execution of selector.includeExecutions) {
    assertAllowed("selector execution", EXECUTIONS, execution, profile.id);
  }
  for (const requirement of selector.excludeRequirements) {
    assertAllowed("selector requirement", REQUIREMENTS, requirement, profile.id);
  }

  profile.selector = selector;
  return profile;
}

function sortEntries(entries) {
  return [...entries].sort((left, right) => left.id.localeCompare(right.id));
}

module.exports = {
  COSTS,
  ENTRYPOINT_TYPES,
  EXECUTIONS,
  ORACLES,
  REQUIREMENTS,
  STABILITIES,
  SURFACES,
  WORLDS,
  sortEntries,
  validateEntry,
  validateProfile,
};
