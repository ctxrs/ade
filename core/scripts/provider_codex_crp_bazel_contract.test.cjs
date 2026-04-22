const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const repoRoot = path.resolve(__dirname, "..", "..");
const protocolBuild = fs.readFileSync(path.join(repoRoot, "core", "crates", "ctx-crp-protocol", "BUILD.bazel"), "utf8");
const codexCrpBuild = fs.readFileSync(path.join(repoRoot, "core", "crates", "codex-crp", "BUILD.bazel"), "utf8");
const providersBuild = fs.readFileSync(path.join(repoRoot, "core", "crates", "ctx-providers", "BUILD.bazel"), "utf8");

test("CRP protocol crate is available to Bazel provider builds", () => {
  assert.match(protocolBuild, /crate_name = "ctx_crp_protocol"/);
  assert.match(protocolBuild, /"@crates\/\/:serde"/);
  assert.match(protocolBuild, /"@crates\/\/:serde_json"/);
});

test("CRP protocol consumers declare the Bazel dependency", () => {
  assert.match(codexCrpBuild, /"\/\/core\/crates\/ctx-crp-protocol:lib"/);
  assert.match(providersBuild, /"\/\/core\/crates\/ctx-crp-protocol:lib"/);
});
