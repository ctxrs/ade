const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");

const {
  DEFAULT_LOCK_STALE_AFTER_MS,
  LOCK_OWNER_FILENAME,
  acquireLock,
  breakStaleLock,
  createRunArtifacts,
  finalizeRunArtifacts,
  getStaleLockObservation,
  normalizeVerificationRunSummary,
  readIndexedSummaries,
} = require("./lib/verification_run_store.cjs");

function tempLayout() {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "verification-run-store-"));
  return {
    artifactsDir: path.join(root, "artifacts"),
  };
}

test("run store finalization writes summaries and prunes by retention count", () => {
  const layout = tempLayout();
  const env = {
    ...process.env,
    CTX_VERIFICATION_RUN_MAX_COUNT: "1",
  };

  const firstRun = createRunArtifacts({
    cwd: process.cwd(),
    env,
    entrypoint: "verify:touched",
    kind: "router",
    layout,
    runId: "run-1",
  });
  finalizeRunArtifacts(firstRun, {
    startedAt: "2026-04-21T00:00:00.000Z",
    completedAt: "2026-04-21T00:00:01.000Z",
    durationMs: 1000,
    success: true,
  }, { env });

  const secondRun = createRunArtifacts({
    cwd: process.cwd(),
    env,
    entrypoint: "verify:affected",
    kind: "router",
    layout,
    runId: "run-2",
  });
  finalizeRunArtifacts(secondRun, {
    startedAt: "2026-04-21T00:00:02.000Z",
    completedAt: "2026-04-21T00:00:03.000Z",
    durationMs: 1000,
    success: true,
  }, { env });

  const summaries = readIndexedSummaries({
    cwd: process.cwd(),
    env,
    layout,
  });

  assert.deepEqual(
    summaries.map((entry) => ({ entrypoint: entry.entrypoint, runId: entry.runId })),
    [{ entrypoint: "verify:affected", runId: "run-2" }],
  );
  assert.equal(fs.existsSync(firstRun.summaryPath), false);
  assert.equal(fs.existsSync(secondRun.summaryPath), true);
});

test("run store normalizes bazel timing fields from phase telemetry", () => {
  const summary = normalizeVerificationRunSummary({
    kind: "bazel",
    durationMs: 1500,
    localSpill: true,
    phases: [
      {
        durationMs: 1000,
        name: "linux-rbe",
        queueTimeMs: 100,
      },
      {
        durationMs: 500,
        name: "local",
        queueTimeMs: 250,
      },
    ],
  });

  assert.equal(summary.queueTimeMs, 350);
  assert.equal(summary.remoteActionTimeMs, 900);
  assert.equal(summary.runnerLocalOverheadMs, 600);
  assert.equal(summary.localSpill, true);
});

test("run store derives local-only bazel telemetry from complete phase data", () => {
  const summary = normalizeVerificationRunSummary({
    kind: "bazel",
    durationMs: 120,
    phases: [
      {
        durationMs: 120,
        name: "local",
        queueTimeMs: 50,
      },
    ],
  });

  assert.equal(summary.queueTimeMs, 50);
  assert.equal(summary.remoteActionTimeMs, 0);
  assert.equal(summary.runnerLocalOverheadMs, 120);
  assert.equal(summary.localSpill, false);
});

test("run store normalizes router timing fields from child bazel runs", () => {
  const layout = tempLayout();
  const run = createRunArtifacts({
    cwd: process.cwd(),
    env: process.env,
    entrypoint: "verify:agent-remote",
    kind: "router",
    layout,
    runId: "router-run",
  });

  const summary = finalizeRunArtifacts(run, {
    childBazelRuns: [
      {
        kind: "bazel",
        durationMs: 1200,
        localSpill: true,
        phases: [
          {
            durationMs: 900,
            name: "linux-rbe",
            queueTimeMs: 100,
          },
          {
            durationMs: 300,
            name: "local",
            queueTimeMs: 50,
          },
        ],
      },
    ],
    completedAt: "2026-04-24T00:00:02.000Z",
    durationMs: 2200,
    remoteSetupMs: 400,
    startedAt: "2026-04-24T00:00:00.000Z",
    success: true,
  }, { env: process.env });

  assert.equal(summary.queueTimeMs, 150);
  assert.equal(summary.remoteActionTimeMs, 800);
  assert.equal(summary.runnerLocalOverheadMs, 800);
  assert.equal(summary.localSpill, true);
  assert.equal(summary.childBazelRuns.length, 1);
  assert.equal(summary.childBazelRuns[0].queueTimeMs, 150);
  assert.equal(summary.childBazelRuns[0].remoteActionTimeMs, 800);
});

test("run store preserves missing optional telemetry instead of coercing defaults", () => {
  const bazelSummary = normalizeVerificationRunSummary({
    kind: "bazel",
    durationMs: 1500,
  });

  assert.equal(bazelSummary.queueTimeMs, undefined);
  assert.equal(bazelSummary.remoteActionTimeMs, undefined);
  assert.equal(bazelSummary.runnerLocalOverheadMs, undefined);
  assert.equal(bazelSummary.localSpill, undefined);

  const routerSummary = normalizeVerificationRunSummary({
    kind: "router",
    durationMs: 2200,
    childBazelRuns: [
      {
        kind: "bazel",
        durationMs: 1200,
        remoteActionTimeMs: 800,
      },
    ],
  });

  assert.equal(routerSummary.queueTimeMs, undefined);
  assert.equal(routerSummary.remoteActionTimeMs, 800);
  assert.equal(routerSummary.runnerLocalOverheadMs, undefined);
  assert.equal(routerSummary.localSpill, undefined);
  assert.equal(routerSummary.childBazelRuns[0].queueTimeMs, undefined);
  assert.equal(routerSummary.childBazelRuns[0].runnerLocalOverheadMs, 400);
  assert.equal(routerSummary.childBazelRuns[0].localSpill, undefined);
});

test("run store preserves explicit zero-valued telemetry", () => {
  const summary = normalizeVerificationRunSummary({
    kind: "bazel",
    durationMs: 1500,
    queueTimeMs: 0,
    remoteActionTimeMs: 0,
    runnerLocalOverheadMs: 0,
    localSpill: false,
  });

  assert.equal(summary.queueTimeMs, 0);
  assert.equal(summary.remoteActionTimeMs, 0);
  assert.equal(summary.runnerLocalOverheadMs, 0);
  assert.equal(summary.localSpill, false);
});

test("run store finalization reclaims stale legacy lock directories", () => {
  const layout = tempLayout();
  const rootDir = path.join(layout.artifactsDir, "verification-runs");
  const lockPath = path.join(rootDir, ".lock");
  fs.mkdirSync(lockPath, { recursive: true });
  const staleAtSeconds = (Date.now() - DEFAULT_LOCK_STALE_AFTER_MS - 1000) / 1000;
  fs.utimesSync(lockPath, staleAtSeconds, staleAtSeconds);

  const run = createRunArtifacts({
    cwd: process.cwd(),
    env: process.env,
    entrypoint: "verify:agent-remote",
    kind: "router",
    layout,
    runId: "stale-legacy-lock",
  });

  const summary = finalizeRunArtifacts(run, {
    startedAt: "2026-04-25T00:00:00.000Z",
    completedAt: "2026-04-25T00:00:01.000Z",
    durationMs: 1000,
    success: true,
  }, { env: process.env });

  assert.equal(summary.runId, "stale-legacy-lock");
  assert.equal(fs.existsSync(lockPath), false);
  assert.equal(fs.existsSync(run.summaryPath), true);
});

test("acquireLock reclaims dead same-host owners without waiting for stale age", () => {
  const rootDir = fs.mkdtempSync(path.join(os.tmpdir(), "verification-lock-"));
  const lockPath = path.join(rootDir, ".lock");
  fs.mkdirSync(lockPath, { recursive: true });
  fs.writeFileSync(path.join(lockPath, LOCK_OWNER_FILENAME), `${JSON.stringify({
    acquiredAt: new Date().toISOString(),
    hostname: os.hostname(),
    pid: 99_999_999,
    token: "dead-owner",
  }, null, 2)}\n`);

  const releaseLock = acquireLock(lockPath, {
    retries: 1,
    retryDelayMs: 0,
    staleAfterMs: 60 * 60 * 1000,
  });

  assert.equal(fs.existsSync(path.join(lockPath, LOCK_OWNER_FILENAME)), true);
  releaseLock();
  assert.equal(fs.existsSync(lockPath), false);
});

test("acquireLock does not break live same-host owners even when the lock looks old", () => {
  const rootDir = fs.mkdtempSync(path.join(os.tmpdir(), "verification-lock-"));
  const lockPath = path.join(rootDir, ".lock");
  fs.mkdirSync(lockPath, { recursive: true });
  fs.writeFileSync(path.join(lockPath, LOCK_OWNER_FILENAME), `${JSON.stringify({
    acquiredAt: "2026-04-01T00:00:00.000Z",
    hostname: os.hostname(),
    pid: process.pid,
    token: "live-owner",
  }, null, 2)}\n`);

  assert.throws(
    () => acquireLock(lockPath, {
      retries: 1,
      retryDelayMs: 0,
      staleAfterMs: 1,
    }),
    /timed out acquiring verification run-store lock/u,
  );
  assert.equal(fs.existsSync(lockPath), true);
  fs.rmSync(lockPath, { recursive: true, force: true });
});

test("acquireLock release does not remove a newer lock owner", () => {
  const rootDir = fs.mkdtempSync(path.join(os.tmpdir(), "verification-lock-"));
  const lockPath = path.join(rootDir, ".lock");
  const releaseLock = acquireLock(lockPath, { retries: 1, retryDelayMs: 0 });
  const ownerPath = path.join(lockPath, LOCK_OWNER_FILENAME);
  const newerOwner = {
    ...JSON.parse(fs.readFileSync(ownerPath, "utf8")),
    token: "newer-owner-token",
  };
  fs.writeFileSync(ownerPath, `${JSON.stringify(newerOwner, null, 2)}\n`);

  releaseLock();

  assert.equal(fs.existsSync(lockPath), true);
  fs.rmSync(lockPath, { recursive: true, force: true });
});

test("breakStaleLock does not delete a lock that was replaced after observation", () => {
  const rootDir = fs.mkdtempSync(path.join(os.tmpdir(), "verification-lock-"));
  const lockPath = path.join(rootDir, ".lock");
  fs.mkdirSync(lockPath, { recursive: true });
  fs.writeFileSync(path.join(lockPath, LOCK_OWNER_FILENAME), `${JSON.stringify({
    acquiredAt: "2026-04-01T00:00:00.000Z",
    hostname: os.hostname(),
    pid: 99_999_999,
    token: "stale-owner",
  }, null, 2)}\n`);

  const staleObservation = getStaleLockObservation(lockPath, {
    staleAfterMs: 60 * 60 * 1000,
  });
  assert.ok(staleObservation);

  fs.rmSync(lockPath, { recursive: true, force: true });
  const releaseLock = acquireLock(lockPath, { retries: 1, retryDelayMs: 0 });

  assert.equal(breakStaleLock(lockPath, staleObservation), false);
  assert.equal(fs.existsSync(lockPath), true);

  releaseLock();
});
