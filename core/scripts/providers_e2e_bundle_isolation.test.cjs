#!/usr/bin/env node

const test = require("node:test");
const assert = require("node:assert/strict");
const path = require("node:path");
const { spawnSync } = require("node:child_process");

const repoRoot = path.resolve(__dirname, "..");
const scriptPath = path.join(repoRoot, "scripts", "providers_e2e.sh");
const canonicalBundleDir = path.join(
  repoRoot,
  "apps",
  "desktop",
  "src-tauri",
  "bundles",
);

test("provider e2e refuses canonical desktop bundle dir mutation by default", () => {
  const result = spawnSync("bash", [scriptPath, "endpoint-ui"], {
    cwd: repoRoot,
    encoding: "utf8",
    env: {
      ...process.env,
      CTX_E2E_BUNDLE_DIR: canonicalBundleDir,
      OPENROUTER_API_KEY: "test-key",
      OPENROUTER_BASE_URL: "https://openrouter.ai/api/v1",
      CTX_E2E_ALLOW_CANONICAL_BUNDLES: "0",
    },
  });

  assert.notEqual(
    result.status,
    0,
    "providers_e2e.sh should fail when CTX_E2E_BUNDLE_DIR points at canonical bundles",
  );
  const output = `${result.stdout || ""}\n${result.stderr || ""}`;
  assert.match(output, /refusing to use canonical desktop bundles dir for e2e/);
});

