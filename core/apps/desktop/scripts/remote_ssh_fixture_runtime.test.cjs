const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { spawnSync } = require("node:child_process");

const repoRoot = path.resolve(__dirname, "..", "..", "..", "..");
const fixtureScript = path.join(repoRoot, "core/apps/desktop/scripts/remote_ssh_fixture.sh");

const parseExportLines = (text) => {
  const env = {};
  for (const rawLine of String(text || "").split(/\r?\n/)) {
    const line = rawLine.trim();
    if (!line.startsWith("export ")) continue;
    const eqIndex = line.indexOf("=");
    if (eqIndex < 0) continue;
    const key = line.slice("export ".length, eqIndex).trim();
    const value = line.slice(eqIndex + 1).trim();
    if (!key) continue;
    env[key] = value.replace(/^'/, "").replace(/'$/, "");
  }
  return env;
};

const run = (cmd, args, options = {}) =>
  spawnSync(cmd, args, {
    cwd: repoRoot,
    encoding: "utf8",
    timeout: options.timeout ?? 240_000,
    ...options,
  });

test("docker ssh fixture defaults nerdctl to the native snapshotter for bind-mounted runs", { timeout: 300_000 }, () => {
  const dockerCheck = spawnSync("docker", ["info", "--format", "{{.ServerVersion}}"], {
    encoding: "utf8",
  });
  if (dockerCheck.status !== 0) {
    test.skip("docker not available");
    return;
  }

  const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-remote-fixture-runtime-"));
  const stateFile = path.join(tmpDir, "fixture.state");
  const logDir = path.join(tmpDir, "logs");
  fs.mkdirSync(logDir, { recursive: true });

  try {
    const start = run("bash", [
      fixtureScript,
      "start",
      "--runtime",
      "docker",
      "--state-file",
      stateFile,
      "--log-dir",
      logDir,
      "--export-container-lane",
    ]);
    assert.equal(start.status, 0, start.stderr);

    const fixtureEnv = parseExportLines(start.stdout);
    const sshConfig = fixtureEnv.CTX_AUTOMATION_REMOTE_FIXTURE_SSH_CONFIG;
    const alias = fixtureEnv.CTX_AUTOMATION_REMOTE_FIXTURE_HOST_ALIAS;
    assert.ok(sshConfig, "fixture should export ssh config path");
    assert.ok(alias, "fixture should export ssh alias");

    const runtimeCheck = run(
      "ssh",
      [
        "-F",
        sshConfig,
        alias,
        "bash -lc 'mkdir -p /tmp/ctx-bind-src && nerdctl info | grep -q \"Storage Driver: native\" && nerdctl run --rm --mount type=bind,src=/tmp/ctx-bind-src,dst=/tmp/ctx-bind-src,rw alpine:3.20 sh -lc \"test -d /tmp/ctx-bind-src && echo fixture-runtime-ok\"'",
      ],
      { timeout: 240_000 },
    );

    assert.equal(runtimeCheck.status, 0, runtimeCheck.stderr);
    assert.match(runtimeCheck.stdout, /fixture-runtime-ok/);
  } finally {
    run("bash", [fixtureScript, "stop", "--state-file", stateFile], { timeout: 120_000 });
    fs.rmSync(tmpDir, { recursive: true, force: true });
  }
});
