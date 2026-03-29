const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");

const repoRoot = path.resolve(__dirname, "..", "..");
const workflowPath = path.join(repoRoot, ".github", "workflows", "macos-avf-exhaustive.yml");

test("macOS AVF exhaustive workflow stays manual+scheduled and targets the dedicated AVF runner group and label", () => {
  const text = fs.readFileSync(workflowPath, "utf8");
  assert.match(text, /workflow_dispatch:/);
  assert.match(text, /schedule:/);
  assert.match(text, /runs-on:\s*\n\s+group:\s*ctx-avf\s*\n\s+labels:\s*ctx-avf-mac-mini/);
  assert.match(text, /Run helper-focused AVF tests/);
  assert.match(text, /Run AVF Linux smoke \(required\)/);
  assert.match(text, /--restore-smoke required/);
  assert.match(text, /ctx-avf-exhaustive\/\*\.json/);
  assert.match(text, /ctx-avf-exhaustive\/\*\.log/);
  assert.match(text, /ctx-avf-exhaustive\/runtime\/version\.txt/);
  assert.match(text, /core\/scripts\/avf_linux_ci_smoke\.sh/);
});
