const childProcess = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");

const {
  collectManagedArchiveTargets,
  verifyManagedArchiveTargetsDetailed,
} = require("../provider_matrix_archive_artifact_gate.cjs");

const BUILD_BUDDY_RELEASE_SCOPES = Object.freeze(["auto", "ctx-only", "providers-only", "both"]);
const DEFAULT_RELEASE_SCOPE = "auto";
const DEFAULT_PROVIDER_MATRIX_RELATIVE_PATH = path.join(
  "core",
  "crates",
  "ctx-provider-accounts",
  "src",
  "provider_matrix.json",
);
const DEFAULT_PROVIDER_DEPS_PROVIDER_IDS = Object.freeze([
  "acp-crp-bridge",
  "amp",
  "claude-crp",
  "droid",
  "goose",
  "opencode",
  "openhands",
  "pi",
]);
const DEFAULT_CODEX_PROVIDER_IDS = Object.freeze(["codex"]);

function normalizeReleaseScope(value) {
  const normalized = String(value || DEFAULT_RELEASE_SCOPE).trim().toLowerCase();
  if (!BUILD_BUDDY_RELEASE_SCOPES.includes(normalized)) {
    throw new Error(
      `unsupported BuildBuddy release scope '${value}'. Allowed scopes: ${BUILD_BUDDY_RELEASE_SCOPES.join(", ")}`,
    );
  }
  return normalized;
}

function releaseScopeIncludesCtx(value) {
  const scope = normalizeReleaseScope(value);
  return scope !== "providers-only";
}

function releaseScopeIncludesProviders(value) {
  const scope = normalizeReleaseScope(value);
  return scope !== "ctx-only";
}

function readProviderMatrixAtCommit({
  commitSha = "",
  matrixRelativePath = DEFAULT_PROVIDER_MATRIX_RELATIVE_PATH,
  repoRoot,
} = {}) {
  const relativePath = String(matrixRelativePath || DEFAULT_PROVIDER_MATRIX_RELATIVE_PATH).trim();
  if (!relativePath) {
    throw new Error("provider matrix path is required");
  }
  if (!repoRoot) {
    throw new Error("repoRoot is required");
  }
  const normalizedCommit = String(commitSha || "").trim();
  if (!normalizedCommit) {
    return JSON.parse(fs.readFileSync(path.join(repoRoot, relativePath), "utf8"));
  }
  const raw = childProcess.execFileSync("git", ["show", `${normalizedCommit}:${relativePath}`], {
    cwd: repoRoot,
    encoding: "utf8",
  });
  return JSON.parse(raw);
}

async function inspectManagedArchiveTargets({
  commitSha = "",
  matrixRelativePath = DEFAULT_PROVIDER_MATRIX_RELATIVE_PATH,
  providerIds = [],
  repoRoot,
  timeoutMs,
} = {}) {
  const matrix = readProviderMatrixAtCommit({
    commitSha,
    matrixRelativePath,
    repoRoot,
  });
  const { missing, targets } = collectManagedArchiveTargets(matrix, providerIds, []);
  if (missing.length > 0) {
    throw new Error(`requested provider ids not present in matrix: ${missing.join(", ")}`);
  }
  if (targets.length === 0) {
    throw new Error("no managed archive targets matched the selected provider set");
  }
  const results = await verifyManagedArchiveTargetsDetailed(targets, {
    timeoutMs,
  });
  return {
    matrix,
    results,
    targets,
    missingArtifactResults: results.filter((result) => result.status === "missing"),
    errorResults: results.filter((result) => result.status === "error"),
    verifiedResults: results.filter((result) => result.status === "verified"),
  };
}

function formatManagedArchiveInspectionSummary({
  label,
  errorResults = [],
  missingArtifactResults = [],
  verifiedResults = [],
} = {}) {
  const lines = [];
  const prefix = String(label || "managed archive verification").trim();
  if (verifiedResults.length > 0) {
    lines.push(`${prefix}: verified ${verifiedResults.length} artifact target(s)`);
  }
  for (const result of missingArtifactResults) {
    lines.push(`${prefix}: missing ${result.providerId}/${result.targetKey}`);
  }
  for (const result of errorResults) {
    lines.push(`${prefix}: ${result.message}`);
  }
  return lines.join("\n");
}

module.exports = {
  BUILD_BUDDY_RELEASE_SCOPES,
  DEFAULT_CODEX_PROVIDER_IDS,
  DEFAULT_PROVIDER_DEPS_PROVIDER_IDS,
  DEFAULT_PROVIDER_MATRIX_RELATIVE_PATH,
  DEFAULT_RELEASE_SCOPE,
  formatManagedArchiveInspectionSummary,
  inspectManagedArchiveTargets,
  normalizeReleaseScope,
  readProviderMatrixAtCommit,
  releaseScopeIncludesCtx,
  releaseScopeIncludesProviders,
};
