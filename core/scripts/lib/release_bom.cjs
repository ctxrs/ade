#!/usr/bin/env node

const fs = require("node:fs");
const path = require("node:path");

const PUBLIC_ARTIFACT_ORIGIN = "https://api.ctx.rs";

function trimValue(value) {
  return String(value || "").trim();
}

function buildPublicStorageUrl({ storageBucket, objectPath } = {}) {
  const normalizedBucket = trimValue(storageBucket);
  const normalizedObjectPath = trimValue(objectPath).replace(/^\/+/, "");
  if (!normalizedBucket || !normalizedObjectPath) {
    throw new Error("storageBucket and objectPath are required");
  }
  return `${PUBLIC_ARTIFACT_ORIGIN}/storage/v1/object/public/${normalizedBucket}/${normalizedObjectPath}`;
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
  storageBucket,
  channel,
  sourceCommit,
} = {}) {
  const normalizedBucket = trimValue(storageBucket);
  const normalizedChannel = trimValue(channel);
  const normalizedCommit = trimValue(sourceCommit);
  if (!normalizedBucket || !normalizedChannel || !normalizedCommit) {
    throw new Error("storageBucket, channel, and sourceCommit are required");
  }
  return buildPublicStorageUrl({
    storageBucket: normalizedBucket,
    objectPath: `providers/${normalizedChannel}/commits/${normalizedCommit}.json`,
  });
}

function buildResolvedProviderManifestDigestUrl({
  storageBucket,
  digestSha256,
} = {}) {
  const normalizedBucket = trimValue(storageBucket);
  const normalizedDigest = trimValue(digestSha256);
  if (!normalizedBucket || !normalizedDigest) {
    throw new Error("storageBucket and digestSha256 are required");
  }
  return buildPublicStorageUrl({
    storageBucket: normalizedBucket,
    objectPath: `providers/manifests/sha256/${normalizedDigest}.json`,
  });
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
  buildPublicStorageUrl,
  buildResolvedProviderManifestDigestUrl,
  buildResolvedProviderManifestPublicUrl,
  PUBLIC_ARTIFACT_ORIGIN,
  writeReleaseBom,
};
