#!/usr/bin/env node

const fs = require("node:fs");
const path = require("node:path");

function trimValue(value) {
  return String(value || "").trim();
}

function normalizeObject(value) {
  if (Array.isArray(value)) {
    return value.map((entry) => normalizeObject(entry));
  }
  if (!value || typeof value !== "object") {
    return value;
  }
  return Object.fromEntries(
    Object.entries(value)
      .filter(([, entry]) => entry !== undefined)
      .map(([key, entry]) => [key, normalizeObject(entry)]),
  );
}

function normalizeRequiredTargets(requiredTargets = []) {
  return [...(Array.isArray(requiredTargets) ? requiredTargets : [])]
    .map((entry) => normalizeObject({
      provider_id: trimValue(entry.providerId || entry.provider_id),
      target_key: trimValue(entry.targetKey || entry.target_key),
      url: trimValue(entry.url),
      sha256: trimValue(entry.expectedSha256 || entry.sha256).toLowerCase(),
      size_bytes: Number.isFinite(entry.sizeBytes) ? Number(entry.sizeBytes) : undefined,
    }))
    .filter((entry) => entry.provider_id || entry.target_key || entry.url || entry.sha256);
}

function normalizeStringArray(values = []) {
  return [...new Set(
    [...(Array.isArray(values) ? values : [])]
      .map((entry) => trimValue(entry))
      .filter(Boolean),
  )];
}

function buildResolvedReleasePlan({
  channel = "",
  intent = {},
  inventory = {},
  preview = {},
  promotion = {},
  providerManifest = {},
  release = {},
  releaseScope = "",
  releaseVersion = "",
  sourceCommit = "",
  storageChannel = "",
  prerequisites = {},
} = {}) {
  const normalizedSourceCommit = trimValue(sourceCommit);
  if (!normalizedSourceCommit) {
    throw new Error("sourceCommit is required");
  }
  const payload = normalizeObject({
    schema_version: 1,
    generated_at: new Date().toISOString(),
    kind: "resolved_release_plan",
    source_commit: normalizedSourceCommit,
    release_version: trimValue(releaseVersion),
    channel: trimValue(channel),
    storage_channel: trimValue(storageChannel),
    release_scope: trimValue(releaseScope),
    provider_manifest: {
      artifact: trimValue(providerManifest.artifact),
      digest_sha256: trimValue(providerManifest.digestSha256 || providerManifest.digest_sha256),
      id: trimValue(providerManifest.id),
      source: trimValue(providerManifest.source),
      source_commit: trimValue(providerManifest.sourceCommit || providerManifest.source_commit),
      url: trimValue(providerManifest.url),
    },
    inventory: {
      required_provider_ids: normalizeStringArray(
        inventory.requiredProviderIds || inventory.required_provider_ids,
      ),
      missing_provider_ids: normalizeStringArray(
        inventory.missingProviderIds || inventory.missing_provider_ids,
      ),
      empty_target_provider_ids: normalizeStringArray(
        inventory.emptyTargetProviderIds || inventory.empty_target_provider_ids,
      ),
      required_targets: normalizeRequiredTargets(
        inventory.requiredTargets || inventory.required_targets,
      ),
    },
    prerequisites,
    preview,
    promotion,
    release,
    intent,
  });
  payload.inventory.target_count = payload.inventory.required_targets.length;
  return payload;
}

function validateResolvedReleasePlan(plan, {
  expectedCommitSha = "",
  requireProviderManifest = false,
} = {}) {
  if (!plan || typeof plan !== "object") {
    throw new Error("resolved release plan must be an object");
  }
  if (trimValue(plan.kind) !== "resolved_release_plan") {
    throw new Error("resolved release plan kind must be 'resolved_release_plan'");
  }
  if (Number(plan.schema_version) !== 1) {
    throw new Error("resolved release plan schema_version must be 1");
  }
  const sourceCommit = trimValue(plan.source_commit);
  if (!sourceCommit) {
    throw new Error("resolved release plan source_commit is required");
  }
  if (trimValue(expectedCommitSha) && trimValue(expectedCommitSha) !== sourceCommit) {
    throw new Error(
      `resolved release plan source_commit mismatch: expected ${trimValue(expectedCommitSha)}, got ${sourceCommit}`,
    );
  }

  const inventory = plan.inventory || {};
  const missingProviderIds = normalizeStringArray(inventory.missing_provider_ids);
  if (missingProviderIds.length > 0) {
    throw new Error(`resolved release plan is missing required providers: ${missingProviderIds.join(", ")}`);
  }
  const emptyTargetProviderIds = normalizeStringArray(inventory.empty_target_provider_ids);
  if (emptyTargetProviderIds.length > 0) {
    throw new Error(
      `resolved release plan has providers without artifact targets: ${emptyTargetProviderIds.join(", ")}`,
    );
  }
  const requiredTargets = normalizeRequiredTargets(inventory.required_targets);
  if (requiredTargets.length === 0) {
    throw new Error("resolved release plan must include at least one required provider target");
  }
  for (const target of requiredTargets) {
    if (!target.provider_id || !target.target_key || !target.url || !target.sha256) {
      throw new Error(
        `resolved release plan target identity is incomplete for ${target.provider_id || "<unknown>"}/${target.target_key || "<unknown>"}`,
      );
    }
  }

  if (requireProviderManifest) {
    const providerManifest = plan.provider_manifest || {};
    if (!trimValue(providerManifest.url)) {
      throw new Error("resolved release plan provider_manifest.url is required");
    }
  }
  return plan;
}

function encodeResolvedReleasePlan(plan) {
  validateResolvedReleasePlan(plan);
  return Buffer.from(JSON.stringify(plan), "utf8").toString("base64");
}

function decodeResolvedReleasePlan(encoded, options = {}) {
  const normalized = trimValue(encoded);
  if (!normalized) {
    throw new Error("resolved release plan payload is required");
  }
  const parsed = JSON.parse(Buffer.from(normalized, "base64").toString("utf8"));
  return validateResolvedReleasePlan(parsed, options);
}

function readResolvedReleasePlan(filePath, options = {}) {
  const resolvedPath = path.resolve(filePath);
  const payload = JSON.parse(fs.readFileSync(resolvedPath, "utf8"));
  return validateResolvedReleasePlan(payload, options);
}

function writeResolvedReleasePlan(filePath, payload) {
  const resolvedPath = path.resolve(filePath);
  const validated = validateResolvedReleasePlan(payload);
  fs.mkdirSync(path.dirname(resolvedPath), { recursive: true });
  fs.writeFileSync(resolvedPath, `${JSON.stringify(validated, null, 2)}\n`, "utf8");
  return resolvedPath;
}

module.exports = {
  buildResolvedReleasePlan,
  decodeResolvedReleasePlan,
  encodeResolvedReleasePlan,
  readResolvedReleasePlan,
  validateResolvedReleasePlan,
  writeResolvedReleasePlan,
};
