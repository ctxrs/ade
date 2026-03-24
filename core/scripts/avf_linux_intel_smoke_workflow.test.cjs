const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");

const repoRoot = path.resolve(__dirname, "..", "..");
const workflowPath = path.join(repoRoot, ".github", "workflows", "avf-linux-intel-smoke.yml");

test("AVF Linux Intel smoke workflow stays manual and pinned to macos-15-intel", () => {
  const text = fs.readFileSync(workflowPath, "utf8");
  assert.match(text, /workflow_dispatch:/);
  assert.match(text, /require_real_smoke:[\s\S]*type: boolean/);
  assert.match(text, /runs-on:\s*macos-15-intel/);
  assert.match(text, /core\/scripts\/avf_linux_ci_smoke\.sh/);
});
