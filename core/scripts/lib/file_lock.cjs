const fs = require("node:fs");
const path = require("node:path");

const DEFAULT_LOCK_POLL_MS = 100;
const DEFAULT_LOCK_STALE_MS = 60 * 60 * 1000;

function parsePid(value) {
  const normalized = Number.parseInt(String(value ?? "").trim(), 10);
  return Number.isFinite(normalized) && normalized > 0 ? normalized : 0;
}

function isProcessAlive(pid, { killImpl = process.kill } = {}) {
  const normalizedPid = parsePid(pid);
  if (!normalizedPid) {
    return false;
  }
  try {
    killImpl(normalizedPid, 0);
    return true;
  } catch (error) {
    return Boolean(error && error.code === "EPERM");
  }
}

function sleepSync(ms) {
  const timeoutMs = Math.max(0, Number(ms) || 0);
  if (timeoutMs === 0) {
    return;
  }
  const sleepArray = new Int32Array(new SharedArrayBuffer(4));
  Atomics.wait(sleepArray, 0, 0, timeoutMs);
}

function readJsonFile(filePath, {
  existsSyncImpl = fs.existsSync,
  readFileSyncImpl = fs.readFileSync,
} = {}) {
  try {
    if (!existsSyncImpl(filePath)) {
      return null;
    }
    return JSON.parse(readFileSyncImpl(filePath, "utf8"));
  } catch {
    return null;
  }
}

function removeFileIfExists(filePath, { rmSyncImpl = fs.rmSync } = {}) {
  try {
    rmSyncImpl(filePath, { force: true });
  } catch {
    // ignore
  }
}

function formatLockHolder(holder) {
  if (!holder || typeof holder !== "object") {
    return "unknown holder";
  }
  const fields = [];
  if (holder.command) {
    fields.push(`command=${holder.command}`);
  }
  if (holder.pid) {
    fields.push(`pid=${holder.pid}`);
  }
  if (holder.leaseId) {
    fields.push(`lease=${holder.leaseId}`);
  }
  if (holder.sessionId) {
    fields.push(`session=${holder.sessionId}`);
  }
  if (holder.threadId) {
    fields.push(`thread=${holder.threadId}`);
  }
  if (holder.acquiredAt) {
    fields.push(`acquiredAt=${holder.acquiredAt}`);
  }
  return fields.length > 0 ? fields.join(" ") : "unknown holder";
}

function inspectExistingLock(lockPath, {
  staleMs = DEFAULT_LOCK_STALE_MS,
  nowMs = Date.now(),
  statSyncImpl = fs.statSync,
  readJsonFileImpl = readJsonFile,
  isProcessAliveImpl = isProcessAlive,
} = {}) {
  let holder = null;
  let stale = false;
  try {
    const lockStat = statSyncImpl(lockPath);
    holder = readJsonFileImpl(lockPath) || null;
    stale = (nowMs - Number(lockStat.mtimeMs || 0)) > staleMs;
  } catch {
    return {
      holder: null,
      stale: true,
    };
  }
  if (!stale && holder && !isProcessAliveImpl(holder.pid)) {
    stale = true;
  }
  return { holder, stale };
}

function tryAcquireFileLockSync(lockPath, {
  metadata = {},
  staleMs = DEFAULT_LOCK_STALE_MS,
  nowMs = Date.now(),
  mkdirSyncImpl = fs.mkdirSync,
  openSyncImpl = fs.openSync,
  writeFileSyncImpl = fs.writeFileSync,
  closeSyncImpl = fs.closeSync,
  removeFileIfExistsImpl = removeFileIfExists,
  inspectExistingLockImpl = inspectExistingLock,
} = {}) {
  mkdirSyncImpl(path.dirname(lockPath), { recursive: true });
  let lockFd = null;
  try {
    lockFd = openSyncImpl(
      lockPath,
      fs.constants.O_CREAT | fs.constants.O_EXCL | fs.constants.O_WRONLY,
      0o600,
    );
    const payload = {
      ...metadata,
      pid: parsePid(metadata.pid) || process.pid,
      acquiredAt: metadata.acquiredAt || new Date(nowMs).toISOString(),
    };
    writeFileSyncImpl(lockFd, `${JSON.stringify(payload, null, 2)}\n`, "utf8");
    return {
      acquired: true,
      holder: payload,
      release() {
        try {
          closeSyncImpl(lockFd);
        } catch {
          // ignore
        }
        removeFileIfExistsImpl(lockPath);
      },
    };
  } catch (error) {
    if (lockFd !== null) {
      try {
        closeSyncImpl(lockFd);
      } catch {
        // ignore
      }
    }
    if (!error || error.code !== "EEXIST") {
      throw error;
    }
  }

  const inspection = inspectExistingLockImpl(lockPath, {
    staleMs,
    nowMs,
  });
  if (inspection.stale) {
    removeFileIfExistsImpl(lockPath);
  }
  return {
    acquired: false,
    holder: inspection.holder,
    stale: inspection.stale,
    release() {},
  };
}

function acquireFileLockSync(lockPath, {
  metadata = {},
  pollMs = DEFAULT_LOCK_POLL_MS,
  staleMs = DEFAULT_LOCK_STALE_MS,
  timeoutMs = 60_000,
  nowImpl = Date.now,
  sleepSyncImpl = sleepSync,
  tryAcquireFileLockSyncImpl = tryAcquireFileLockSync,
} = {}) {
  const deadline = nowImpl() + Math.max(0, Number(timeoutMs) || 0);
  let lastHolder = null;
  while (true) {
    const attempt = tryAcquireFileLockSyncImpl(lockPath, {
      metadata,
      staleMs,
      nowMs: nowImpl(),
    });
    if (attempt.acquired) {
      return attempt;
    }
    lastHolder = attempt.holder;
    if (nowImpl() >= deadline) {
      throw new Error(
        `timed out acquiring lock ${lockPath} after ${timeoutMs}ms (${formatLockHolder(lastHolder)})`,
      );
    }
    sleepSyncImpl(pollMs);
  }
}

function withFileLockSync(lockPath, options, fn) {
  const lock = acquireFileLockSync(lockPath, options);
  try {
    return fn(lock);
  } finally {
    lock.release();
  }
}

module.exports = {
  DEFAULT_LOCK_POLL_MS,
  DEFAULT_LOCK_STALE_MS,
  acquireFileLockSync,
  formatLockHolder,
  inspectExistingLock,
  isProcessAlive,
  parsePid,
  readJsonFile,
  removeFileIfExists,
  sleepSync,
  tryAcquireFileLockSync,
  withFileLockSync,
};
