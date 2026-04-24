const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const coreRoot = path.resolve(__dirname, "..");
const repoRoot = path.resolve(coreRoot, "..");
const packageJson = JSON.parse(fs.readFileSync(path.join(coreRoot, "package.json"), "utf8"));
const installSiteBuild = fs.readFileSync(path.join(repoRoot, "install-site", "BUILD.bazel"), "utf8");
const installBootstrapRunner = fs.readFileSync(
  path.join(repoRoot, "scripts", "buildbuddy", "run_install_bootstrap_contracts.sh"),
  "utf8",
);

test("install-site contract lane routes through Bazel-owned deterministic tests", () => {
  assert.equal(
    packageJson.scripts["bazel:install-site:test"],
    "node scripts/run_bazel_pilot.cjs test //install-site:install_contract_tests",
  );
  assert.match(installSiteBuild, /load\("@aspect_rules_js\/\/js:defs\.bzl", "js_test"\)/);
  assert.match(installSiteBuild, /name = "install_route_contract_tests"/);
  assert.match(installSiteBuild, /name = "install_script_contract_tests"/);
  assert.match(installSiteBuild, /name = "install_bootstrap_contract_tests"/);
  assert.match(installSiteBuild, /name = "install_contract_tests"/);
  assert.match(installSiteBuild, /name = "install_route_contract_tests"[\s\S]*?entry_point = "src\/run-node-test-suite\.mjs"[\s\S]*?args = \["src\/index\.test\.mjs"\]/);
  assert.match(installSiteBuild, /name = "install_script_contract_tests"[\s\S]*?entry_point = "src\/run-node-test-suite\.mjs"[\s\S]*?"src\/install-script\.test\.mjs"[\s\S]*?"src\/uninstall-script\.test\.mjs"/);
  assert.match(installSiteBuild, /name = "install_bootstrap_contract_tests"[\s\S]*?entry_point = "src\/run-node-test-suite\.mjs"[\s\S]*?args = \["src\/install-bootstrap-ci\.test\.mjs"\]/);
  assert.doesNotMatch(installSiteBuild, /run_workspace_task\.sh/);
  assert.match(installSiteBuild, /"src\/run-node-test-suite\.mjs"/);
  assert.match(installSiteBuild, /"src\/index\.test\.mjs"/);
  assert.match(installSiteBuild, /"src\/install-script\.test\.mjs"/);
  assert.match(installSiteBuild, /"src\/uninstall-script\.test\.mjs"/);
  assert.match(installSiteBuild, /"src\/install-bootstrap-ci\.test\.mjs"/);
  assert.doesNotMatch(installSiteBuild, /\/\/core:package\.json/);
  assert.match(installBootstrapRunner, /pnpm -C core install --frozen-lockfile/);
  assert.match(installBootstrapRunner, /node core\/scripts\/run_bazel_pilot\.cjs test \/\/install-site:install_contract_tests/);
});
