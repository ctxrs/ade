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
const bundleHarnessProvidersScriptPath = path.join(
  repoRoot,
  "..",
  "scripts",
  "lib",
  "bundled_harnesses_providers.sh",
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
test("linux-arm lanes source repo-owned local adapters from the workspace before publish", () => {
  const script = fs.readFileSync(scriptPath, "utf8");

  assert.match(
    script,
    /bundle_provider_csv="acp-crp-bridge,\$\{bundle_provider_csv\}"/,
  );
  assert.match(
    script,
    /CTX_E2E_ENDPOINT_BUNDLE_INCLUDE_BRIDGE="\$\{CTX_E2E_ENDPOINT_BUNDLE_INCLUDE_BRIDGE:-1\}"/,
  );
  assert.match(
    script,
    /source local adapters from the workspace while external lane/,
  );
  assert.match(
    script,
    /providers continue using their managed archive paths/,
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
  assert.match(
    script,
    /matrix_payload_json="\$\(node "\$\{repo_root\}\/scripts\/linux_arm_provider_reliability_matrix\.cjs" --lane "\$\{lane_key\}" --json\)"/,
  );
  assert.match(
    script,
    /CTX_E2E_INSTALL_SMOKE_ENVIRONMENT="\$\{CTX_E2E_INSTALL_SMOKE_ENVIRONMENT:-\$\{expected_environment:-host\}\}"/,
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
  const providersScript = fs.readFileSync(
    bundleHarnessProvidersScriptPath,
    "utf8",
  );

  assert.match(
    script,
    /source "\$ROOT\/scripts\/lib\/bundled_harnesses_providers\.sh"/,
  );
  assert.match(providersScript, /elif kind == "archive":/);
  assert.match(
    providersScript,
    /bin_path\.endswith\(\("\.js", "\.mjs", "\.cjs"\)\)/,
  );
});
