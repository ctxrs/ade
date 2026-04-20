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

function buildResolvedProviderManifestPublicUrl({
  supabaseUrl,
  storageBucket,
  channel,
  sourceCommit,
} = {}) {
  const normalizedUrl = trimValue(supabaseUrl).replace(/\/+$/, "");
  const normalizedBucket = trimValue(storageBucket);
  const normalizedChannel = trimValue(channel);
  const normalizedCommit = trimValue(sourceCommit);
  if (!normalizedUrl || !normalizedBucket || !normalizedChannel || !normalizedCommit) {
    throw new Error("supabaseUrl, storageBucket, channel, and sourceCommit are required");
  }
  return `${normalizedUrl}/storage/v1/object/public/${normalizedBucket}/providers/${normalizedChannel}/commits/${normalizedCommit}.json`;
}

function buildReleaseBom({
  channel = "",
  intent = {},
  preview = {},
  providerManifest = {},
  releaseScope = "",
  releaseVersion = "",
  sourceCommit = "",
  stageArtifacts = [],
  storageChannel = "",
  prerequisites = {},
} = {}) {
  const normalizedSourceCommit = trimValue(sourceCommit);
  if (!normalizedSourceCommit) {
    throw new Error("sourceCommit is required");
  }
  return normalizeObject({
    schema_version: 1,
    generated_at: new Date().toISOString(),
    kind: "resolved_release_manifest",
    source_commit: normalizedSourceCommit,
    release_version: trimValue(releaseVersion),
    channel: trimValue(channel),
    storage_channel: trimValue(storageChannel),
    release_scope: trimValue(releaseScope),
    provider_manifest: normalizeObject({
      artifact: trimValue(providerManifest.artifact),
      digest_sha256: trimValue(providerManifest.digestSha256),
      id: trimValue(providerManifest.id),
      source: trimValue(providerManifest.source),
      url: trimValue(providerManifest.url),
    }),
    prerequisites,
    stage_artifacts: Array.isArray(stageArtifacts) ? stageArtifacts.map((entry) => normalizeObject(entry)) : [],
    preview,
    intent,
  });
}

function writeReleaseBom(filePath, payload) {
  const resolvedPath = path.resolve(filePath);
  fs.mkdirSync(path.dirname(resolvedPath), { recursive: true });
  fs.writeFileSync(resolvedPath, `${JSON.stringify(payload, null, 2)}\n`, "utf8");
  return resolvedPath;
}

module.exports = {
  buildReleaseBom,
  buildResolvedProviderManifestPublicUrl,
  writeReleaseBom,
};
