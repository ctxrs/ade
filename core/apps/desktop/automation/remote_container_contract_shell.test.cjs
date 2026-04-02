const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");

const specPath = path.join(
  __dirname,
  "specs",
  "remote-container-contract.spec.cjs",
);

test("remote container contract runs SSH commands under bash -lc", () => {
  const script = fs.readFileSync(specPath, "utf8");

  assert.match(script, /const remoteCommand = `bash -lc \$\{JSON\.stringify\(command\)\}`;/);
  assert.match(script, /fixture\.target,\s*remoteCommand,\s*\];/);
  assert.match(script, /finalArgs = \[\.\.\.args, "-o", "BatchMode=yes", fixture\.target, remoteCommand\];/);
  assert.match(script, /remote_command: remoteCommand,/);
});
