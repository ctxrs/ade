const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const childProcess = require("node:child_process");

const scriptPath = path.resolve(__dirname, "bundled_dependency_updates.cjs");
const scriptText = fs.readFileSync(scriptPath, "utf8");
const repoRoot = path.resolve(__dirname, "..", "..");

test("bundled dependency policy reads managed runtime versions from ctx-managed-installs", () => {
  assert.match(scriptText, /ctx-managed-installs", "src", "lib\.rs"/);
});

test("bundled dependency policy mode resolves current runtime constants without network access", () => {
  const result = childProcess.spawnSync(process.execPath, [scriptPath, "policy"], {
    cwd: repoRoot,
    encoding: "utf8",
  });
  assert.equal(result.status, 0, result.stderr || result.stdout);
  assert.match(result.stdout, /^runtime:ok\tnode\t/m);
  assert.match(result.stdout, /^runtime:ok\tpython\t/m);
  assert.match(result.stdout, /^runtime:ok\tpython-build-tag\t/m);
});
