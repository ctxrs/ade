const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const {
  createRemoteContractRecorder,
  resolveRemoteFixtureEnv,
} = require("./helpers/remote_fixture_contract.cjs");

test("host fixture resolution derives user from user@host and enforces the shared contract", () => {
  const fixture = resolveRemoteFixtureEnv({
    lane: "host",
    env: {
      CTX_AUTOMATION_REMOTE_HOST: "alice@example.com",
      CTX_AUTOMATION_REMOTE_DATA_DIR: "/tmp/ctx-remote-host",
      CTX_AUTOMATION_REMOTE_STRICT: "1",
    },
  });

  assert.equal(fixture.host, "example.com");
  assert.equal(fixture.user, "alice");
  assert.equal(fixture.target, "alice@example.com");
  assert.equal(fixture.wizardHostInput, "alice@example.com");
  assert.equal(fixture.port, 44099);
  assert.equal(fixture.dataDir, "/tmp/ctx-remote-host");
  assert.equal(fixture.ready, true);
  assert.equal(fixture.strictRequired, true);
  assert.deepEqual(fixture.missingRequirements, []);
});

test("sandbox fixture resolution falls back to base env and allow-skip disables strict failures", () => {
  const fixture = resolveRemoteFixtureEnv({
    lane: "sandbox",
    env: {
      CTX_AUTOMATION_REMOTE_HOST: "builder.example",
      CTX_AUTOMATION_REMOTE_USER: "builder",
      CTX_AUTOMATION_REMOTE_DATA_DIR: "/tmp/ctx-remote-host",
      CTX_AUTOMATION_REMOTE_CONTAINER_DATA_DIR: "/tmp/ctx-remote-container",
      CTX_AUTOMATION_REMOTE_STRICT: "1",
      CTX_AUTOMATION_REMOTE_ALLOW_SKIP: "1",
    },
  });

  assert.equal(fixture.host, "builder.example");
  assert.equal(fixture.user, "builder");
  assert.equal(fixture.port, 44099);
  assert.equal(fixture.dataDir, "/tmp/ctx-remote-container");
  assert.equal(fixture.ready, true);
  assert.equal(fixture.strictRequired, false);
  assert.equal(fixture.allowSkip, true);
});

test("remote contract recorder redacts secrets in artifacts and ssh transcripts", () => {
  const outputDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-remote-contract-recorder-"));
  const reportPath = path.join(outputDir, "report.json");
  try {
    const fixture = resolveRemoteFixtureEnv({
      lane: "host",
      env: {
        CTX_AUTOMATION_REMOTE_HOST: "alice@example.com",
        CTX_AUTOMATION_REMOTE_DATA_DIR: "/tmp/ctx-remote-host",
      },
    });
    const recorder = createRemoteContractRecorder({
      outputPath: reportPath,
      suite: "remote bootstrap install e2e",
      lane: "host",
      fixture,
      secretValues: ["topsecret", "sk-live-123"],
    });

    recorder.recordArtifact("provider", {
      detail: "topsecret leaked",
      token: "sk-live-123",
    });
    recorder.recordSshTranscript("remote-daemon-log-tail", {
      stdout: "daemon topsecret line",
      stderr: "provider key sk-live-123",
    });
    const report = recorder.finalize({
      result: "failed",
      reason: "topsecret failed",
      error: "sk-live-123 failed",
    });
    const reportText = fs.readFileSync(reportPath, "utf8");

    assert.equal(report.result, "failed");
    assert.match(reportText, /\[REDACTED\]/);
    assert.doesNotMatch(reportText, /topsecret/);
    assert.doesNotMatch(reportText, /sk-live-123/);
  } finally {
    fs.rmSync(outputDir, { recursive: true, force: true });
  }
});
