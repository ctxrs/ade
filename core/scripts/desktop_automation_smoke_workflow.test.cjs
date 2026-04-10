#!/usr/bin/env node

const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const repoRoot = path.resolve(__dirname, "..", "..");
const workflowPath = path.join(repoRoot, ".github", "workflows", "desktop-automation-smoke.yml");

test("desktop automation smoke labels the prepared remote bootstrap lane as fast regression only", () => {
  const workflow = fs.readFileSync(workflowPath, "utf8");

  assert.match(workflow, /desktop-remote-bootstrap-fixture:\s*\n\s+name:\s+Desktop remote bootstrap fixture \(fast regression only\)/);
  assert.match(workflow, /Desktop remote bootstrap fixture auth matrix \(fast regression only\)/);
  assert.match(workflow, /desktop-remote-bootstrap-fast-regression-\$\{\{ github\.run_id \}\}/);
  assert.doesNotMatch(workflow, /Desktop remote bootstrap fixture \(release gate\)/);
});

test("desktop automation smoke includes runtime surfaces that affect local AVF sandbox creation", () => {
  const workflow = fs.readFileSync(workflowPath, "utf8");

  for (const pattern of [
    /core\/apps\/desktop\/src-tauri\/src\/avf_linux_helper\/\*\*/,
    /core\/crates\/ctx-avf-linux-runtime\/\*\*/,
    /core\/crates\/ctx-http\/\*\*/,
    /core\/crates\/ctx-sandbox-container-runtime\/\*\*/,
    /core\/crates\/ctx-workspace-container\/\*\*/,
  ]) {
    assert.match(workflow, pattern);
  }
});

test("desktop automation smoke runs a real macOS AVF first-run wizard lane", () => {
  const workflow = fs.readFileSync(workflowPath, "utf8");

  assert.match(workflow, /desktop-first-run-macos-avf:\s*\n\s+runs-on:\s+\$\{\{\s*vars\.RELEASE_RUNNER_MACOS_ARM64 \|\| 'macos-14'\s*\}\}/);
  assert.match(workflow, /name:\s+Desktop first-run wizard smoke \(local AVF sandbox\)/);
  assert.match(workflow, /CTX_AUTOMATION_SCENARIOS:\s+local-avf-first-run/);
  assert.match(workflow, /CTX_AUTOMATION_FIRST_RUN_CONFIRM_DELAY_MS:\s+10000/);
  assert.match(workflow, /pnpm -C core\/apps\/desktop run test:automation:first-run-local-sandbox/);
});
