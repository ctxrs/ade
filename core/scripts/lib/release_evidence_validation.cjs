#!/usr/bin/env node

const crypto = require("node:crypto");

function stableStringify(value) {
  if (Array.isArray(value)) {
    return `[${value.map((entry) => stableStringify(entry)).join(",")}]`;
  }
  if (!value || typeof value !== "object") {
    return JSON.stringify(value);
  }
  return `{${Object.keys(value)
    .filter((key) => value[key] !== undefined)
    .sort()
    .map((key) => `${JSON.stringify(key)}:${stableStringify(value[key])}`)
    .join(",")}}`;
}

function buildReleasePlanEvidenceDigest(plan) {
  if (!plan || typeof plan !== "object" || Array.isArray(plan)) {
    throw new Error("resolved release plan must be an object");
  }
  return crypto.createHash("sha256").update(stableStringify(plan)).digest("hex");
}

module.exports = {
  buildReleasePlanEvidenceDigest,
  stableStringify,
};
