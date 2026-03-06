const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const childProcess = require("node:child_process");

const repoRoot = path.resolve(__dirname, "..", "..");
const scriptPath = path.join(repoRoot, "scripts", "updater_e2e_localssh.sh");

const run = (args) =>
  childProcess.spawnSync("bash", [scriptPath, ...args], {
    cwd: repoRoot,
    encoding: "utf8",
    env: {
      ...process.env,
    },
  });

test("localssh provider manages ssh key state", () => {
  if (childProcess.spawnSync("bash", ["-lc", "command -v docker"], { encoding: "utf8" }).status !== 0) {
    return;
  }

  const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-localssh-test-"));
  const stateFile = path.join(tempDir, "state.json");
  const pubKeyFile = path.join(tempDir, "id_ed25519.pub");
  fs.writeFileSync(pubKeyFile, "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIMatrixFixtureKey local-test\n", "utf8");

  const create = run([
    "create-ssh-key",
    "--name",
    "ctx-local-test-key",
    "--public-key-file",
    pubKeyFile,
    "--state-file",
    stateFile,
  ]);
  assert.equal(create.status, 0, `stdout=${create.stdout}\nstderr=${create.stderr}`);
  const created = JSON.parse(create.stdout.trim());
  assert.ok(created.id);

  const list = run([
    "list-ssh-keys",
    "--state-file",
    stateFile,
  ]);
  assert.equal(list.status, 0, `stdout=${list.stdout}\nstderr=${list.stderr}`);
  const listed = JSON.parse(list.stdout.trim());
  assert.equal(listed.count, 1);

  const leak = run([
    "leak-check",
    "--run-id",
    "no-such-run",
    "--state-file",
    stateFile,
  ]);
  assert.equal(leak.status, 0, `stdout=${leak.stdout}\nstderr=${leak.stderr}`);
  const leaked = JSON.parse(leak.stdout.trim());
  assert.equal(leaked.count, 0);

  const del = run([
    "delete-ssh-key",
    "--ssh-key-id",
    created.id,
    "--state-file",
    stateFile,
  ]);
  assert.equal(del.status, 0, `stdout=${del.stdout}\nstderr=${del.stderr}`);

  fs.rmSync(tempDir, { recursive: true, force: true });
});
