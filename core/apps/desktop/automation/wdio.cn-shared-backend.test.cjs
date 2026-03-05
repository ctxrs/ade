const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const childProcess = require("node:child_process");

const stateDir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-cn-shared-backend-test-"));
process.env.CTX_AUTOMATION_CN_BACKEND_STATE_DIR = stateDir;
process.env.CTX_AUTOMATION_CN_STOP_SHARED_BACKEND_WHEN_IDLE = "0";
const hooks = require("./wdio.conf.cjs").__cnSharedBackendTestHooks;

if (!hooks) {
  throw new Error("missing __cnSharedBackendTestHooks export");
}

const nowIso = () => new Date().toISOString();
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

const isProcessAlive = (pid) => {
  try {
    process.kill(pid, 0);
    return true;
  } catch {
    return false;
  }
};

const resetStateDir = () => {
  fs.rmSync(stateDir, { recursive: true, force: true });
  fs.mkdirSync(stateDir, { recursive: true });
};

const waitForProcessExit = async (pid, timeoutMs = 2000) => {
  const started = Date.now();
  while (Date.now() - started < timeoutMs) {
    if (!isProcessAlive(pid)) return true;
    await sleep(25);
  }
  return !isProcessAlive(pid);
};

test.after(() => {
  fs.rmSync(stateDir, { recursive: true, force: true });
});

test("lease lifecycle writes active lease and release removes it", async () => {
  resetStateDir();
  const leaseId = hooks.createLeaseId();
  const leasePath = hooks.leasePath(leaseId);

  await hooks.withLock(async () => {
    hooks.writeJsonFileAtomic(leasePath, {
      leaseId,
      pid: process.pid,
      host: "127.0.0.1",
      port: 3000,
      createdAt: nowIso(),
      updatedAt: nowIso(),
    });
  });

  const activeBefore = hooks.cleanupStaleLeases();
  assert.equal(activeBefore.some((entry) => entry.leaseId === leaseId), true);

  hooks.setCurrentLeaseId(leaseId);
  await hooks.releaseSharedCnBackendLease("127.0.0.1", 3000);

  const activeAfter = hooks.cleanupStaleLeases();
  assert.equal(activeAfter.some((entry) => entry.leaseId === leaseId), false);
  assert.equal(fs.existsSync(leasePath), false);
});

test("stale lease cleanup removes dead-pid lease file", async () => {
  resetStateDir();
  const staleProc = childProcess.spawn(
    process.execPath,
    ["-e", "setInterval(() => {}, 1000)"],
    { stdio: "ignore" },
  );
  staleProc.kill("SIGKILL");
  const exited = await waitForProcessExit(staleProc.pid);
  assert.equal(exited, true);

  const leaseId = hooks.createLeaseId();
  const leasePath = hooks.leasePath(leaseId);
  await hooks.withLock(async () => {
    hooks.writeJsonFileAtomic(leasePath, {
      leaseId,
      pid: staleProc.pid,
      host: "127.0.0.1",
      port: 3000,
      createdAt: nowIso(),
      updatedAt: nowIso(),
    });
  });

  const active = hooks.cleanupStaleLeases();
  assert.equal(active.some((entry) => entry.leaseId === leaseId), false);
  assert.equal(fs.existsSync(leasePath), false);
});

test("shared backend release leaves backend process alive when stop-when-idle is disabled", async () => {
  resetStateDir();
  const backendProc = childProcess.spawn(
    process.execPath,
    ["-e", "setInterval(() => {}, 1000)"],
    { stdio: "ignore" },
  );

  const leaseId = hooks.createLeaseId();
  const leasePath = hooks.leasePath(leaseId);
  await hooks.withLock(async () => {
    hooks.writeJsonFileAtomic(leasePath, {
      leaseId,
      pid: process.pid,
      host: "127.0.0.1",
      port: 3000,
      createdAt: nowIso(),
      updatedAt: nowIso(),
    });
    hooks.writeState({
      host: "127.0.0.1",
      port: 3000,
      pid: backendProc.pid,
      startedAt: nowIso(),
      updatedAt: nowIso(),
    });
  });

  assert.equal(isProcessAlive(backendProc.pid), true);

  hooks.setCurrentLeaseId(leaseId);
  await hooks.releaseSharedCnBackendLease("127.0.0.1", 3000);

  assert.equal(isProcessAlive(backendProc.pid), true);
  assert.equal(fs.existsSync(leasePath), false);
  assert.equal(fs.existsSync(hooks.getPaths().stateFile), true);

  backendProc.kill("SIGKILL");
});
