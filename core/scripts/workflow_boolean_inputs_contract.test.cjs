const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");

const repoRoot = path.resolve(__dirname, "..", "..");

const workflowText = (name) =>
  fs.readFileSync(path.join(repoRoot, ".github", "workflows", name), "utf8");

test("desktop break matrix manual uses a typed boolean include_remote input", () => {
  const text = workflowText("desktop-break-matrix-manual.yml");
  assert.match(text, /include_remote:[\s\S]*type: boolean/);
  assert.match(text, /INPUT_INCLUDE_REMOTE:\s*\$\{\{\s*inputs\.include_remote && '1' \|\| '0'\s*\}\}/);
});

test("updater janitor uses a typed boolean dry_run input", () => {
  const text = workflowText("updater-e2e-janitor.yml");
  assert.match(text, /dry_run:[\s\S]*type: boolean/);
  assert.ok(!text.includes("inputs.dry_run == 'true'"));
});

test("publish claude-crp workflow uses a typed boolean publish input", () => {
  const text = workflowText("publish-claude-crp-provider-deps.yml");
  assert.match(text, /publish_to_supabase:[\s\S]*type: boolean/);
  assert.match(text, /if:\s*\$\{\{\s*github\.event_name == 'workflow_dispatch' && inputs\.publish_to_supabase\s*\}\}/);
});

test("release preflight uses a typed boolean override input", () => {
  const text = workflowText("release-preflight.yml");
  assert.match(text, /linux_arm_critical_override:[\s\S]*type: boolean/);
  assert.ok(!text.includes("type: choice"));
  assert.match(text, /1\|true\|yes\|on/);
});

test("release supabase uses typed updater drill booleans", () => {
  const text = workflowText("release-supabase.yml");
  assert.match(text, /run_updater_e2e_drill:[\s\S]*type: boolean/);
  assert.match(text, /updater_e2e_expect_version_change:[\s\S]*type: boolean/);
  assert.ok(!text.includes("inputs.run_updater_e2e_drill == 'true'"));
  assert.match(text, /CTX_UPDATER_E2E_EXPECT_VERSION_CHANGE:\s*\$\{\{\s*github\.event_name == 'workflow_dispatch'/);
});
