import { test } from "node:test";
import assert from "node:assert/strict";
import * as path from "node:path";
import { pathToFileURL } from "node:url";

const rootDir = path.resolve(import.meta.dirname, "..");

async function loadRuntimeModule() {
  return import(pathToFileURL(path.join(rootDir, "dist", "runtime.js")).href);
}

const supportedModels = [
  {
    value: "default",
    displayName: "Default (recommended)",
    description: "Sonnet 4.6 · Best for everyday tasks",
    supportsEffort: true,
    supportedEffortLevels: ["low", "medium", "high", "max"],
  },
  {
    value: "opus",
    displayName: "Opus",
    description: "Opus 4.7 · Most capable for complex work · ~2× usage vs Sonnet",
    supportsEffort: true,
    supportedEffortLevels: ["low", "medium", "high", "xhigh", "max"],
  },
  {
    value: "haiku",
    displayName: "Haiku",
    description: "Haiku 4.5 · Fastest for quick answers",
  },
];

test("buildClaudeModelsListEnvelope expands Claude models into versioned labels and efforts", async () => {
  const { buildClaudeModelsListEnvelope } = await loadRuntimeModule();

  const payload = buildClaudeModelsListEnvelope({ supportedModels });
  const entriesById = new Map(payload.models.map((entry) => [entry.id, entry.name]));

  assert.equal(payload.currentModelId, "default/high");
  assert.equal(entriesById.get("default/high"), "Default (Sonnet 4.6) (High)");
  assert.equal(entriesById.get("opus/xhigh"), "Opus 4.7 (XHigh)");
  assert.equal(entriesById.get("haiku"), "Haiku 4.5");
  assert.ok(entriesById.has("default/max"), "expected default/max effort variant");
  assert.ok(entriesById.has("opus/max"), "expected opus/max effort variant");
});

test("buildClaudeModelsListEnvelope preserves an explicitly selected concrete Claude slug", async () => {
  const { buildClaudeModelsListEnvelope } = await loadRuntimeModule();

  const payload = buildClaudeModelsListEnvelope({
    supportedModels,
    requestedModel: "claude-opus-4-7",
    requestedReasoningEffort: "xhigh",
  });
  const selected = payload.models.find((entry) => entry.id === "claude-opus-4-7/xhigh");

  assert.equal(payload.currentModelId, "claude-opus-4-7/xhigh");
  assert.deepEqual(selected, {
    id: "claude-opus-4-7/xhigh",
    name: "Opus 4.7 (XHigh)",
  });
});
