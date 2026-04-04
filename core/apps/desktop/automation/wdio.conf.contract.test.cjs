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

test("wdio shipped-app mode is cross-platform and supports an explicit bundle dir override", () => {
  const script = fs.readFileSync(configPath, "utf8");

  assert.match(script, /const SHIPPED_APP_BUNDLES_DIR_OVERRIDE = resolveConfiguredPath\(\s*process\.env\.CTX_AUTOMATION_SHIPPED_APP_BUNDLES_DIR,\s*\)/);
  assert.match(script, /const USING_SHIPPED_APP_MODE = SHIPPED_APP_MODE;/);
  assert.match(script, /if \(USING_SHIPPED_APP_MODE\) \{\s*if \(SHIPPED_APP_BUNDLES_DIR_OVERRIDE\) \{\s*return SHIPPED_APP_BUNDLES_DIR_OVERRIDE;/s);
  assert.match(
    script,
    /CTX_AUTOMATION_SHIPPED_APP=1 requires CTX_AUTOMATION_SHIPPED_APP_BUNDLES_DIR for Linux\/Windows container scenarios/,
  );
  assert.match(
    script,
    /delete process\.env\.CTX_DESKTOP_DEV_BIN_DIR;[\s\S]*delete process\.env\.CTX_DESKTOP_START_PATH;/,
  );
});

test("wdio remote-only prep on mac arm64 skips the managed AVF payload download", () => {
  const script = fs.readFileSync(configPath, "utf8");

  assert.match(script, /const RUNS_REMOTE_ONLY_SCENARIOS = SCENARIO_FILTER\.length > 0/);
  assert.match(script, /SCENARIO_FILTER\.every\(\(token\) => token\.startsWith\("remote"\)\)/);
  assert.match(
    script,
    /process\.platform === "darwin"[\s\S]*process\.arch === "arm64"[\s\S]*RUNS_REMOTE_ONLY_SCENARIOS[\s\S]*CTX_DESKTOP_ALLOW_MANAGED_AVF_RUNTIME_MISSING_LOCAL_PAYLOAD = "1"/,
  );
});
