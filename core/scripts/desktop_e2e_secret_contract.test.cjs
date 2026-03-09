const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const childProcess = require("node:child_process");

const scriptPath = path.join(__dirname, "desktop_e2e_secret_contract.cjs");

const run = (args) =>
  childProcess.spawnSync("node", [scriptPath, ...args], {
    cwd: path.join(__dirname, ".."),
    encoding: "utf8",
  });

test("contract report writes and then validates cleanly", () => {
  const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-desktop-e2e-contract-"));
  const reportPath = path.join(tempDir, "ci_preflight_secret_contract.md");

  const writeResult = run(["--report", reportPath]);
  assert.equal(writeResult.status, 0, `stdout=${writeResult.stdout}\nstderr=${writeResult.stderr}`);
  assert.match(writeResult.stdout, /wrote contract report/i);

  const report = fs.readFileSync(reportPath, "utf8");
  assert.match(report, /provider-auth-matrix-required/);
  assert.match(report, /CTX_E2E_CODEX_OAUTH_EMAIL/);
  assert.match(report, /CTX_E2E_CODEX_OAUTH_PASSWORD/);
  assert.match(report, /mac-webdriver-quick-smoke/);

  const checkResult = run(["--check-report", reportPath]);
  assert.equal(checkResult.status, 0, `stdout=${checkResult.stdout}\nstderr=${checkResult.stderr}`);
});

test("contract report check fails when file content is stale", () => {
  const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-desktop-e2e-contract-stale-"));
  const reportPath = path.join(tempDir, "ci_preflight_secret_contract.md");
  fs.writeFileSync(reportPath, "# stale\n", "utf8");

  const result = run(["--check-report", reportPath]);
  assert.notEqual(result.status, 0, `stdout=${result.stdout}\nstderr=${result.stderr}`);
  assert.match(result.stderr, /out of date/i);
});
