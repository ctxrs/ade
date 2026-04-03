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
