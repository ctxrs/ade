#!/usr/bin/env node

const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const configPath = path.join(__dirname, "wdio.conf.cjs");

test("wdio automation defaults route runtime and daemon scratch through the volatile root", () => {
  const script = fs.readFileSync(configPath, "utf8");

  assert.match(script, /resolveVolatileRoot = \(\) =>/);
  assert.match(script, /path\.join\(os\.homedir\(\), "\.ctx", "volatile"\)/);
  assert.match(script, /AUTOMATION_ARTIFACTS_ROOT = resolveVolatileSubdir\("CTX_VOLATILE_ARTIFACTS_DIR", \["artifacts"\]\)/);
  assert.match(script, /resolveAutomationTmpRoot = \(\) =>/);
  assert.match(script, /process\.env\.TMPDIR = automationTmpDir/);
  assert.match(script, /return path\.join\(AUTOMATION_ARTIFACTS_ROOT, "ctx-desktop-e2e", "avf-linux-guest-runtime"\)/);
  assert.match(script, /fs\.mkdtempSync\(path\.join\(automationTmpDir, `ctx-desktop-e2e-\$\{daemonPort\}-`\)\)/);
  assert.match(script, /fs\.mkdtempSync\(path\.join\(automationTmpDir, "ctx-desktop-e2e-app-daemon-"\)\)/);
});
