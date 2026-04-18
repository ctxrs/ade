const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");

const {
  acquireFileLockSync,
  withFileLockSync,
} = require("./file_lock.cjs");

test("withFileLockSync creates and removes the lock file around the callback", () => {
  const rootDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-file-lock-"));
  const lockPath = path.join(rootDir, "lease.lock");
  let observedDuringCallback = false;

  withFileLockSync(lockPath, {
    metadata: {
      command: "test-lock",
    },
  }, () => {
    observedDuringCallback = fs.existsSync(lockPath);
  });

  assert.equal(observedDuringCallback, true);
  assert.equal(fs.existsSync(lockPath), false);
});

test("acquireFileLockSync reclaims stale lock files", () => {
  const rootDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-file-lock-stale-"));
  const lockPath = path.join(rootDir, "lease.lock");
  fs.mkdirSync(path.dirname(lockPath), { recursive: true });
  fs.writeFileSync(lockPath, `${JSON.stringify({ pid: 999999 })}\n`, "utf8");

  const lock = acquireFileLockSync(lockPath, {
    metadata: {
      command: "reclaim-stale",
    },
    staleMs: 60_000,
    timeoutMs: 5,
  });
  lock.release();

  assert.equal(fs.existsSync(lockPath), false);
});

test("acquireFileLockSync reports the current holder on timeout", () => {
  const rootDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-file-lock-timeout-"));
  const lockPath = path.join(rootDir, "lease.lock");
  fs.mkdirSync(path.dirname(lockPath), { recursive: true });
  fs.writeFileSync(
    lockPath,
    `${JSON.stringify({ pid: process.pid, command: "already-held", leaseId: "lease-123" })}\n`,
    "utf8",
  );

  assert.throws(
    () => acquireFileLockSync(lockPath, {
      metadata: {
        command: "blocked",
      },
      staleMs: 60_000,
      timeoutMs: 0,
    }),
    /already-held/,
  );
});
