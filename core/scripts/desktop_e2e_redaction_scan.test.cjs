const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const childProcess = require("node:child_process");

const scriptPath = path.join(__dirname, "desktop_e2e_redaction_scan.cjs");

const run = (args, env = {}) =>
  childProcess.spawnSync("node", [scriptPath, ...args], {
    cwd: path.join(__dirname, ".."),
    encoding: "utf8",
    env: {
      ...process.env,
      ...env,
    },
  });

test("redaction scan fails on a seeded secret assignment leak", () => {
  const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-desktop-redaction-fail-"));
  const logPath = path.join(tempDir, "run.log");
  fs.writeFileSync(logPath, "OPENROUTER_API_KEY=openrouter_secret_value_12345\n", "utf8");

  const result = run([
    "--suite",
    "providers-endpoint-ui",
    "--path",
    logPath,
  ]);

  assert.notEqual(result.status, 0, `stdout=${result.stdout}\nstderr=${result.stderr}`);
  assert.match(result.stderr, /OPENROUTER_API_KEY/);
  assert.match(result.stderr, /redaction scan failed/i);
});

test("redaction scan honors allowlist rows with justification", () => {
  const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-desktop-redaction-pass-"));
  const logPath = path.join(tempDir, "run.log");
  const allowlistPath = path.join(tempDir, "allowlist.tsv");
  fs.writeFileSync(logPath, "OPENROUTER_API_KEY=openrouter_secret_value_12345\n", "utf8");
  fs.writeFileSync(
    allowlistPath,
    "run\\.log\tOPENROUTER_API_KEY=openrouter_secret_value_12345\tfixture false positive for scanner coverage\n",
    "utf8",
  );

  const result = run([
    "--suite",
    "providers-endpoint-ui",
    "--path",
    logPath,
    "--allowlist",
    allowlistPath,
  ]);

  assert.equal(result.status, 0, `stdout=${result.stdout}\nstderr=${result.stderr}`);
  assert.match(result.stdout, /redaction scan passed/i);
});
