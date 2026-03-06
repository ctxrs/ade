#!/usr/bin/env node

const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { spawnSync } = require("node:child_process");

const repoRoot = path.resolve(__dirname, "..");
const scriptPath = path.join(repoRoot, "scripts", "providers_e2e.sh");
const bundleHarnessScriptPath = path.join(
  repoRoot,
  "..",
  "scripts",
  "ensure_bundled_harnesses.sh",
);
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
      OPENROUTER_API_KEY: "openrouter_secret_value_12345",
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
test("linux-arm lanes bundle acp-crp-bridge via managed provider artifacts", () => {
  const script = fs.readFileSync(scriptPath, "utf8");

  assert.match(
    script,
    /bundle_provider_csv="acp-crp-bridge,\$\{bundle_provider_csv\}"/,
  );
  assert.match(
    script,
    /CTX_E2E_ENDPOINT_BUNDLE_INCLUDE_BRIDGE="\$\{CTX_E2E_ENDPOINT_BUNDLE_INCLUDE_BRIDGE:-0\}"/,
  );
  assert.match(
    script,
    /managed archive bundling path as the other lane providers/,
  );
});

test("linux-arm lanes rely on host podman instead of demanding a bundled podman archive", () => {
  const script = fs.readFileSync(scriptPath, "utf8");

  assert.match(
    script,
    /runtime lock only vendors Podman artifacts for/,
  );
  assert.match(
    script,
    /macOS remote clients/,
  );
  assert.match(
    script,
    /CTX_E2E_ENDPOINT_BUNDLE_PODMAN="\$\{CTX_E2E_ENDPOINT_BUNDLE_PODMAN:-0\}"/,
  );
});

test("endpoint bundle defaults use a home-shareable cache root", () => {
  const script = fs.readFileSync(scriptPath, "utf8");

  assert.match(script, /default_e2e_bundle_root\(\)/);
  assert.match(script, /Library\/Caches\/ctx-e2e/);
  assert.match(
    script,
    /bundle_dir="\$\{CTX_E2E_BUNDLE_DIR:-\$\{cache_root\}\/bundles-\$\{cache_key\}\}"/,
  );
});

test("bundle harnesses treat archive js entrypoints as requiring node", () => {
  const script = fs.readFileSync(bundleHarnessScriptPath, "utf8");

  assert.match(script, /elif kind == "archive":/);
  assert.match(script, /bin_path\.endswith\(\("\.js", "\.mjs", "\.cjs"\)\)/);
});
