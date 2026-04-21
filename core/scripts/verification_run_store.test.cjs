const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");

const {
  createRunArtifacts,
  finalizeRunArtifacts,
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
