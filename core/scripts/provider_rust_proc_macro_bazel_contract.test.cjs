const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const repoRoot = path.resolve(__dirname, "..", "..");
const acpCrpBridgeBuild = fs.readFileSync(
  path.join(repoRoot, "external-harnesses", "acp-crp-bridge", "BUILD.bazel"),
  "utf8",
);
const droidAcpBuild = fs.readFileSync(path.join(repoRoot, "harness-adapters", "droid-acp", "BUILD.bazel"), "utf8");

test("Rust provider adapters keep async-trait in proc_macro_deps", () => {
  const acpDepsBlock = acpCrpBridgeBuild.match(/ACP_CRP_BRIDGE_DEPS = \[(.*?)\]\n\nACP_CRP_BRIDGE_PROC_MACRO_DEPS/s);
  const droidDepsBlock = droidAcpBuild.match(/DROID_ACP_DEPS = \[(.*?)\]\n\nDROID_ACP_PROC_MACRO_DEPS/s);

  assert.ok(acpDepsBlock, "acp-crp-bridge deps block should stay structurally recognizable");
  assert.ok(droidDepsBlock, "droid-acp deps block should stay structurally recognizable");
  assert.match(acpCrpBridgeBuild, /ACP_CRP_BRIDGE_PROC_MACRO_DEPS = \[\s+"@crates\/\/:async-trait",\s+\]/);
  assert.doesNotMatch(acpDepsBlock[1], /@crates\/\/:async-trait/);
  assert.match(acpCrpBridgeBuild, /proc_macro_deps = ACP_CRP_BRIDGE_PROC_MACRO_DEPS/);

  assert.match(droidAcpBuild, /DROID_ACP_PROC_MACRO_DEPS = \[\s+"@crates\/\/:async-trait",\s+\]/);
  assert.doesNotMatch(droidDepsBlock[1], /@crates\/\/:async-trait/);
  assert.match(droidAcpBuild, /proc_macro_deps = DROID_ACP_PROC_MACRO_DEPS/);
});
