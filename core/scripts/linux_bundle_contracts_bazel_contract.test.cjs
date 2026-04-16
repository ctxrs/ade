const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const repoRoot = path.resolve(__dirname, "..", "..");
const toolsBuild = fs.readFileSync(path.join(repoRoot, "tools", "bazel", "BUILD.bazel"), "utf8");
const helperScript = fs.readFileSync(path.join(repoRoot, "tools", "bazel", "linux_bundle_contracts.sh"), "utf8");

test("linux bundle contracts expose a Bazel-owned release wrapper", () => {
  assert.match(toolsBuild, /"linux_bundle_contracts\.sh"/);
  assert.match(toolsBuild, /name = "linux_bundle_contracts"/);
  assert.match(helperScript, /BUILD_WORKSPACE_DIRECTORY/);
  assert.match(helperScript, /scripts\/linux_bundle_prune_glibc\.sh/);
  assert.match(helperScript, /scripts\/linux_bundle_gate\.sh/);
  assert.match(helperScript, /--bundles-dir "\$bundles_dir" --mode both/);
  assert.match(helperScript, /--prune-glibc/);
});
