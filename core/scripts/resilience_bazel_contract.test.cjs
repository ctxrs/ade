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

function targetBlock(buildText, name) {
  const lines = buildText.split(/\r?\n/u);
  const nameLineIndex = lines.findIndex((line) => line.trim() === `name = "${name}",`);
  assert.notEqual(nameLineIndex, -1, `expected target ${name}`);

  let startIndex = nameLineIndex;
  while (startIndex > 0 && !/^[A-Za-z0-9_.]+\($/u.test(lines[startIndex].trim())) {
    startIndex -= 1;
  }
  assert.match(lines[startIndex].trim(), /^[A-Za-z0-9_.]+\($/u, `expected start of target ${name}`);

  let endIndex = nameLineIndex;
  while (endIndex < lines.length && lines[endIndex].trim() !== ")") {
    endIndex += 1;
  }
  assert.notEqual(endIndex, lines.length, `expected end of target ${name}`);

  return lines.slice(startIndex, endIndex + 1).join("\n");
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
  const desktopIpcCorpusBlock = targetBlock(webBuild, "desktop_ipc_corpus");
  assert.match(webBuild, /name = "desktop_ipc_corpus"/);
  assert.match(desktopIpcCorpusBlock, /^vitest_bin\.vitest_test\(/m);
  assert.doesNotMatch(desktopIpcCorpusBlock, /run_workspace_task\.sh/);
});
