#!/usr/bin/env node

const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const { spawnSync } = require("node:child_process");

const repoRoot = path.resolve(__dirname, "..");
const scriptPath = path.join(repoRoot, "scripts", "providers_e2e.sh");
const webE2eServerPath = path.join(repoRoot, "apps", "web", "scripts", "start-e2e-server.mjs");
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
    /if \[\[ "\$\{CTX_BUNDLE_BUILD_CODEX_CRP:-0\}" == "0" \]\]; then[\s\S]*bundled_only_provider_csv="\$\(csv_remove_provider "\$\{bundled_only_provider_csv\}" "codex"\)"/,
  );
  assert.match(
    script,
    /source local adapters from the workspace while external lane/,
  );
  assert.match(
    script,
    /providers continue using their managed archive paths/,
  );
  assert.match(
    script,
    /enforcing bundled-only runtime resolution for: \$\{CTX_E2E_BUNDLED_ONLY_PROVIDERS\}/,
  );
});

test("linux-arm lanes keep sandbox runtime bundling out of the default host-mode path", () => {
  const script = fs.readFileSync(scriptPath, "utf8");

  assert.match(
    script,
    /run_linux_arm_runtime_install_lane/,
  );
  assert.match(
    script,
    /CTX_E2E_ENDPOINT_APPEND_LINUX_TARGETS="\$\{CTX_E2E_ENDPOINT_APPEND_LINUX_TARGETS:-0\}"/,
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

test("web provider lanes invoke the checked-in apps/web playwright binary", () => {
  const script = fs.readFileSync(scriptPath, "utf8");

  assert.match(
    script,
    /run_web_playwright_suite\(\)/,
  );
  assert.match(
    script,
    /playwright_bin="\$\{web_root\}\/node_modules\/\.bin\/playwright"/,
  );
  assert.match(
    script,
    /ensuring locked playwright install in \$\{web_root\}/,
  );
  assert.match(
    script,
    /pnpm install --frozen-lockfile >\/dev\/null/,
  );
  assert.match(
    script,
    /cd "\$\{web_root\}"/,
  );
  assert.match(
    script,
    /node -e "require\.resolve\('playwright\/package\.json'\)"/,
  );
  assert.match(
    script,
    /missing playwright binary in \$\{web_root\} after locked install; run 'bash -lc \\"cd \$\{web_root\} && pnpm install --frozen-lockfile\\"'/,
  );
  assert.match(
    script,
    /playwright package is not resolvable from \$\{web_root\} after locked install; run 'bash -lc \\"cd \$\{web_root\} && pnpm install --frozen-lockfile\\"'/,
  );
});

test("endpoint-ui lane defaults to host-mode bundles and scopes focused reruns", () => {
  const script = fs.readFileSync(scriptPath, "utf8");

  assert.match(
    script,
    /if \[\[ -n "\$\{CTX_E2E_ENDPOINT_PROVIDERS:-\}" && -z "\$\{CTX_E2E_ENDPOINT_BUNDLE_PROVIDERS:-\}" \]\]; then/,
  );
  assert.match(
    script,
    /endpoint_bundle_providers="acp-crp-bridge,\$\{endpoint_bundle_providers\}"/,
  );
  assert.match(
    script,
    /CTX_E2E_ENDPOINT_BUNDLE_HARNESS_IMAGE="\$\{CTX_E2E_ENDPOINT_BUNDLE_HARNESS_IMAGE:-0\}"/,
  );
  assert.match(
    script,
    /CTX_E2E_ENDPOINT_APPEND_LINUX_TARGETS="\$\{CTX_E2E_ENDPOINT_APPEND_LINUX_TARGETS:-0\}"/,
  );
  assert.match(
    script,
    /local first_pass_providers="\$\{CTX_E2E_ENDPOINT_BUNDLE_PROVIDERS:-acp-crp-bridge,codex,cline,copilot,gemini,goose,openhands,qwen,pi,opencode,mistral,droid,kimi\}"/,
  );
  assert.match(
    script,
    /if \[\[ -n "\$\{CTX_E2E_ENDPOINT_SPECS:-\}" \]\]; then/,
  );
  assert.match(
    script,
    /IFS=',' read -r -a endpoint_specs <<<"\$\{CTX_E2E_ENDPOINT_SPECS\}"/,
  );
});

test("tokens lane builds and exports the local worktree bridge binary", () => {
  const script = fs.readFileSync(scriptPath, "utf8");

  assert.match(
    script,
    /local_bridge_manifest="\$\{repo_root\}\/\.\.\/external-harnesses\/acp-crp-bridge\/Cargo\.toml"/,
  );
  assert.match(
    script,
    /CTX_TOKENS_ACP_CRP_BRIDGE_BIN="\$\{CTX_TOKENS_ACP_CRP_BRIDGE_BIN:-\$\{CTX_E2E_CARGO_TARGET_DIR\}\/debug\/acp-crp-bridge\}"/,
  );
  assert.match(
    script,
    /cargo build --manifest-path "\$\{local_bridge_manifest\}" --bin acp-crp-bridge >\/dev\/null/,
  );
  assert.match(
    script,
    /missing local acp-crp-bridge binary after build: \$\{CTX_TOKENS_ACP_CRP_BRIDGE_BIN\}/,
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

test("bundle harnesses pin cargo artifacts under the bundle build dir by default", () => {
  const script = fs.readFileSync(bundleHarnessScriptPath, "utf8");

  assert.match(script, /CARGO_TARGET_DIR="\$bundle_build_dir\/cargo\/\$target_key"/);
  assert.match(script, /export CARGO_TARGET_DIR/);
});

test("bundle runtime downloads use mktemp templates that work on macOS", () => {
  const script = fs.readFileSync(bundleHarnessScriptPath, "utf8");

  assert.match(script, /mktemp -p "\$dest_dir" "node-\$\{node_folder\}\.XXXXXX"/);
  assert.match(script, /mktemp -p "\$dest_dir" "python-\$\{py_folder\}\.XXXXXX"/);
  assert.doesNotMatch(script, /mktemp -p "\$dest_dir" "node-[^"]+XXXXXX\.[^"]+"/);
  assert.doesNotMatch(script, /mktemp -p "\$dest_dir" "python-[^"]+XXXXXX\.[^"]+"/);
});

test("bundle archive extraction dispatches on detected archive type, not temp filename suffix", () => {
  const script = fs.readFileSync(bundleHarnessScriptPath, "utf8");

  assert.match(script, /local archive_path=""\s+local archive_type=""/);
  assert.match(script, /case "\$archive_type" in/);
  assert.match(script, /archive_type="zip"/);
  assert.match(script, /archive_type="tar"/);
  assert.match(script, /archive_type="tgz"/);
});

test("web e2e daemon launcher injects an explicit ctx-mcp command", () => {
  const script = fs.readFileSync(webE2eServerPath, "utf8");

  assert.match(script, /const ensureCtxMcpCommand = \(repoRoot, env\) =>/);
  assert.match(script, /runSync\(cargoCmd, \["build", "-p", "ctx-mcp", "--bin", "ctx-mcp"\]/);
  assert.match(script, /const binaryPath = path\.join\(resolveCargoTargetDir\(repoRoot, env\), "debug", binName\)/);
  assert.match(script, /env\.CTX_MCP_COMMAND = ensureCtxMcpCommand\(repoRoot, env\);/);
});
