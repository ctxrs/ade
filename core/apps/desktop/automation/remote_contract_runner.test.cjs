const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { spawnSync } = require("node:child_process");

const repoRoot = path.resolve(__dirname, "..", "..", "..", "..");
const runnerPath = path.join(repoRoot, "core/apps/desktop/scripts/test_remote_real_ci.sh");

const runRunner = (args, env = {}) =>
  spawnSync("bash", [runnerPath, ...args], {
    cwd: repoRoot,
    env: { ...process.env, ...env },
    encoding: "utf8",
  });

test("remote contracts runner dry-run writes preflight and lane summary artifacts", () => {
  const artifactDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-remote-runner-dry-run-"));
  try {
    const result = runRunner(
      ["--dry-run", "--artifacts-dir", artifactDir],
      {
        CTX_AUTOMATION_REMOTE_HOST: "builder@example.com",
        CTX_AUTOMATION_REMOTE_DATA_DIR: "/tmp/ctx-remote-host",
        CTX_AUTOMATION_REMOTE_CONTAINER_DATA_DIR: "/tmp/ctx-remote-container",
        CTX_AUTOMATION_REMOTE_FIXTURE_CLASS: "docker-ssh",
        CTX_AUTOMATION_REMOTE_FIXTURE_SANDBOX_RUNTIME: "nested-containerd",
      },
    );

    assert.equal(result.status, 0, result.stderr);
    const preflight = JSON.parse(fs.readFileSync(path.join(artifactDir, "preflight.json"), "utf8"));
    const summary = fs.readFileSync(path.join(artifactDir, "summary.tsv"), "utf8");

    assert.equal(preflight.lanes.host.ready, true);
    assert.equal(preflight.lanes.container.ready, true);
    assert.equal(preflight.lanes.host.proofScope, "docker_warm_remote_host");
    assert.equal(preflight.lanes.container.proofScope, "docker_warm_remote_sandbox");
    assert.match(summary, /remote-host\tdry-run\t0/);
    assert.match(summary, /remote-container\tdry-run\t0/);
  } finally {
    fs.rmSync(artifactDir, { recursive: true, force: true });
  }
});

test("remote contracts runner fails strict preflight with explicit missing env output", () => {
  const artifactDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-remote-runner-preflight-"));
  try {
    const result = runRunner(["--dry-run", "--artifacts-dir", artifactDir], {});

    assert.equal(result.status, 2);
    assert.match(result.stderr, /host remote fixture is missing required configuration/);
    assert.match(result.stderr, /CTX_AUTOMATION_REMOTE_HOST/);
    assert.match(result.stderr, /CTX_AUTOMATION_REMOTE_DATA_DIR/);
    assert.equal(fs.existsSync(path.join(artifactDir, "preflight.json")), true);
  } finally {
    fs.rmSync(artifactDir, { recursive: true, force: true });
  }
});
