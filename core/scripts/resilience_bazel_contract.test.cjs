const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const coreRoot = path.resolve(__dirname, "..");
const packageJson = JSON.parse(fs.readFileSync(path.join(coreRoot, "package.json"), "utf8"));
const ctxCoreBuild = fs.readFileSync(path.join(coreRoot, "crates", "ctx-core", "BUILD.bazel"), "utf8");
const ctxHttpBazelTests = fs.readFileSync(path.join(coreRoot, "crates", "ctx-http", "ctx_http_bazel_tests.bzl"), "utf8");
const ctxMcpBuild = fs.readFileSync(path.join(coreRoot, "crates", "ctx-mcp", "BUILD.bazel"), "utf8");
const ctxProvidersBuild = fs.readFileSync(path.join(coreRoot, "crates", "ctx-providers", "BUILD.bazel"), "utf8");
const ctxStoreBuild = fs.readFileSync(path.join(coreRoot, "crates", "ctx-store", "BUILD.bazel"), "utf8");
const webBuild = fs.readFileSync(path.join(coreRoot, "apps", "web", "BUILD.bazel"), "utf8");

function targetBlock(buildText, name, nextName) {
  const start = buildText.indexOf(`name = "${name}"`);
  assert.notEqual(start, -1, `expected target ${name}`);
  const end = buildText.indexOf(`name = "${nextName}"`, start + 1);
  assert.notEqual(end, -1, `expected target ${nextName}`);
  return buildText.slice(start, end);
}

test("split resilience lanes use existing Bazel targets where they already exist", () => {
  assert.equal(
    packageJson.scripts["bazel:anomaly:ctx-http:fault-matrix"],
    "node scripts/run_bazel_pilot.cjs test //core/crates/ctx-http:fault_matrix",
  );
  assert.equal(
    packageJson.scripts["bazel:anomaly:ctx-http:hot-endpoints-no-db"],
    "node scripts/run_bazel_pilot.cjs test //core/crates/ctx-http:hot_endpoints_no_db",
  );
  assert.equal(
    packageJson.scripts["bazel:fuzz:workspace-payloads"],
    "node scripts/run_bazel_pilot.cjs test //core/crates/ctx-core:workspace_payload_corpus",
  );
  assert.equal(
    packageJson.scripts["bazel:anomaly:ctx-store:fault-injection"],
    "node scripts/run_bazel_pilot.cjs test //core/crates/ctx-store:unit_tests_fault_injection",
  );
  assert.equal(
    packageJson.scripts["bazel:fuzz:providers"],
    "node scripts/run_bazel_pilot.cjs test //core/crates/ctx-providers:unit_tests_fuzz",
  );
  assert.equal(
    packageJson.scripts["bazel:fuzz:mcp"],
    "node scripts/run_bazel_pilot.cjs test //core/crates/ctx-mcp:fuzz_tests",
  );
  assert.equal(
    packageJson.scripts["bazel:fuzz:release-manifests"],
    "node scripts/run_bazel_pilot.cjs test //core/crates/ctx-http:release_manifest_corpus",
  );
  assert.equal(
    packageJson.scripts["bazel:fuzz:desktop-ipc"],
    "node scripts/run_bazel_pilot.cjs test //core/apps/web:desktop_ipc_corpus",
  );

  assert.match(ctxCoreBuild, /name = "workspace_payload_corpus"/);
  assert.match(ctxHttpBazelTests, /"fault_matrix"/);
  assert.match(ctxHttpBazelTests, /"hot_endpoints_no_db"/);
  assert.match(ctxHttpBazelTests, /"release_manifest_corpus"/);
  assert.match(ctxMcpBuild, /name = "fuzz_tests"/);
  assert.match(ctxMcpBuild, /crate_features = \["fuzz_tests"\]/);
  assert.match(ctxProvidersBuild, /name = "unit_tests_fuzz"/);
  assert.match(ctxProvidersBuild, /crate_features = \["fuzz_tests"\]/);
  assert.match(ctxStoreBuild, /name = "unit_tests_fault_injection"/);
  assert.match(ctxStoreBuild, /crate_features = \["fault_injection"\]/);
  const desktopIpcCorpusBlock = targetBlock(webBuild, "desktop_ipc_corpus", "e2e_premerge");
  assert.match(webBuild, /name = "desktop_ipc_corpus"/);
  assert.match(webBuild, /vitest_bin\.vitest_test\([\s\S]*?name = "desktop_ipc_corpus"/);
  assert.doesNotMatch(desktopIpcCorpusBlock, /run_workspace_task\.sh/);
});
