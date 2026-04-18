const os = require("node:os");
const path = require("node:path");

const { resolveCtxCacheLayout } = require("./cache_roots.cjs");
const {
  formatLockHolder,
  sleepSync,
  tryAcquireFileLockSync,
} = require("./file_lock.cjs");

const ACTIVE_BUDGETS_ENV = "CTX_HOST_JOB_ACTIVE_BUDGETS";
const LEASE_ID_ENV = "CTX_HOST_JOB_LEASE_ID";
const DISABLE_BUDGETS_ENV = "CTX_HOST_JOB_BUDGETS_DISABLED";
const DEFAULT_BUDGET_POLL_MS = 100;
const DEFAULT_BUDGET_TIMEOUT_MS = 20 * 60 * 1000;
const DEFAULT_BUDGET_STALE_MS = 6 * 60 * 60 * 1000;
const HOST_HEAVY_BUDGET_KEY = "host-heavy";
const DESKTOP_PREPARE_BUDGET_KEY = "desktop-prepare";

function trimValue(value) {
  return String(value ?? "").trim();
}

function parseBool(value) {
  return ["1", "true", "yes", "on"].includes(trimValue(value).toLowerCase());
}

function normalizeBudgetKey(value) {
  return trimValue(value).toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-+|-+$/g, "");
}

function computeDefaultHeavyBudgetSlots(cpuCount = os.availableParallelism?.() || os.cpus().length) {
  const cores = Math.max(1, Number(cpuCount) || 1);
  if (cores <= 4) {
    return 1;
  }
  if (cores <= 8) {
    return 2;
  }
  if (cores <= 16) {
    return 3;
  }
  return 4;
}

function parsePositiveInteger(value) {
  const normalized = trimValue(value);
  if (!/^\d+$/.test(normalized)) {
    return null;
  }
  const parsed = Number.parseInt(normalized, 10);
  return parsed > 0 ? parsed : null;
}

function resolveBudgetEnvKey(budgetKey) {
  return `CTX_HOST_JOB_BUDGET_${normalizeBudgetKey(budgetKey).replace(/-/g, "_").toUpperCase()}_SLOTS`;
}

function resolveHostBudgetConfig(budgetKey, {
  env = process.env,
  cpuCount = os.availableParallelism?.() || os.cpus().length,
} = {}) {
  const normalizedBudgetKey = normalizeBudgetKey(budgetKey);
  if (!normalizedBudgetKey) {
    throw new Error("host job budget key is required");
  }
  const slotsOverride = parsePositiveInteger(env[resolveBudgetEnvKey(normalizedBudgetKey)]);
  const defaults = {
    [HOST_HEAVY_BUDGET_KEY]: computeDefaultHeavyBudgetSlots(cpuCount),
    [DESKTOP_PREPARE_BUDGET_KEY]: 1,
  };
  return {
    budgetKey: normalizedBudgetKey,
    slots: slotsOverride ?? defaults[normalizedBudgetKey] ?? 1,
    pollMs: parsePositiveInteger(env.CTX_HOST_JOB_BUDGET_POLL_MS) ?? DEFAULT_BUDGET_POLL_MS,
    staleMs: parsePositiveInteger(env.CTX_HOST_JOB_BUDGET_STALE_MS) ?? DEFAULT_BUDGET_STALE_MS,
    timeoutMs: parsePositiveInteger(env.CTX_HOST_JOB_BUDGET_TIMEOUT_MS) ?? DEFAULT_BUDGET_TIMEOUT_MS,
  };
}

function resolveHostBudgetRoot({ cwd = process.cwd(), env = process.env } = {}) {
  const layout = resolveCtxCacheLayout({ cwd, env });
  return path.join(layout.cacheDir, "host-job-budgets");
}

function parseActiveBudgets(env = process.env) {
  return new Set(
    trimValue(env[ACTIVE_BUDGETS_ENV])
      .split(",")
      .map((entry) => normalizeBudgetKey(entry))
      .filter(Boolean),
  );
}

function buildLeaseId(env = process.env) {
  return trimValue(env[LEASE_ID_ENV])
    || `${process.pid}-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
}

function updateActiveBudgetsEnv(env, activeBudgets) {
  const serialized = [...activeBudgets].sort().join(",");
  if (serialized) {
    env[ACTIVE_BUDGETS_ENV] = serialized;
    return;
  }
  delete env[ACTIVE_BUDGETS_ENV];
}

function buildHostJobLeaseMetadata({
  budgetKey,
  command = "",
  cwd = process.cwd(),
  env = process.env,
  leaseId = buildLeaseId(env),
  slotIndex = null,
} = {}) {
  return {
    budgetKey: normalizeBudgetKey(budgetKey),
    command: trimValue(command),
    cwd: path.resolve(cwd),
    leaseId,
    pid: process.pid,
    sessionId: trimValue(env.CTX_SESSION_ID),
    slotIndex,
    threadId: trimValue(env.CODEX_THREAD_ID),
  };
}

function withHostJobBudget({
  budgetKey,
  command = "",
  cwd = process.cwd(),
  env = process.env,
  budgetRoot = resolveHostBudgetRoot({ cwd, env }),
  pollMs = null,
  sleepSyncImpl = sleepSync,
  slots = null,
  staleMs = null,
  timeoutMs = null,
  tryAcquireFileLockSyncImpl = tryAcquireFileLockSync,
}, fn) {
  if (typeof fn !== "function") {
    throw new Error("withHostJobBudget requires a callback");
  }
  if (parseBool(env[DISABLE_BUDGETS_ENV])) {
    return fn();
  }

  const config = resolveHostBudgetConfig(budgetKey, { env });
  const normalizedBudgetKey = config.budgetKey;
  const activeBudgets = parseActiveBudgets(env);
  if (activeBudgets.has(normalizedBudgetKey)) {
    return fn();
  }

  const totalSlots = slots ?? config.slots;
  if (!Number.isFinite(totalSlots) || totalSlots <= 0) {
    return fn();
  }

  const deadline = Date.now() + (timeoutMs ?? config.timeoutMs);
  const effectivePollMs = pollMs ?? config.pollMs;
  const effectiveStaleMs = staleMs ?? config.staleMs;
  const leaseId = buildLeaseId(env);
  let acquiredLock = null;
  let holders = [];

  while (!acquiredLock) {
    holders = [];
    for (let slotIndex = 0; slotIndex < totalSlots; slotIndex += 1) {
      const lockPath = path.join(budgetRoot, normalizedBudgetKey, `slot-${slotIndex}.lock`);
      const attempt = tryAcquireFileLockSyncImpl(lockPath, {
        metadata: buildHostJobLeaseMetadata({
          budgetKey: normalizedBudgetKey,
          command,
          cwd,
          env,
          leaseId,
          slotIndex,
        }),
        staleMs: effectiveStaleMs,
      });
      if (attempt.acquired) {
        acquiredLock = {
          ...attempt,
          slotIndex,
        };
        break;
      }
      if (attempt.holder) {
        holders.push(attempt.holder);
      }
    }
    if (acquiredLock) {
      break;
    }
    if (Date.now() >= deadline) {
      const holderSummary = holders.length > 0
        ? holders.map((holder) => formatLockHolder(holder)).join("; ")
        : "no current holder metadata";
      throw new Error(
        `timed out acquiring host budget '${normalizedBudgetKey}' after ${timeoutMs ?? config.timeoutMs}ms (${holderSummary})`,
      );
    }
    sleepSyncImpl(effectivePollMs);
  }

  const previousLeaseId = env[LEASE_ID_ENV];
  const previousActiveBudgets = env[ACTIVE_BUDGETS_ENV];
  activeBudgets.add(normalizedBudgetKey);
  env[LEASE_ID_ENV] = leaseId;
  updateActiveBudgetsEnv(env, activeBudgets);

  try {
    return fn(acquiredLock);
  } finally {
    acquiredLock.release();
    if (previousLeaseId) {
      env[LEASE_ID_ENV] = previousLeaseId;
    } else {
      delete env[LEASE_ID_ENV];
    }
    if (previousActiveBudgets) {
      env[ACTIVE_BUDGETS_ENV] = previousActiveBudgets;
    } else {
      delete env[ACTIVE_BUDGETS_ENV];
    }
  }
}

module.exports = {
  ACTIVE_BUDGETS_ENV,
  DEFAULT_BUDGET_POLL_MS,
  DEFAULT_BUDGET_STALE_MS,
  DEFAULT_BUDGET_TIMEOUT_MS,
  DESKTOP_PREPARE_BUDGET_KEY,
  DISABLE_BUDGETS_ENV,
  HOST_HEAVY_BUDGET_KEY,
  LEASE_ID_ENV,
  buildHostJobLeaseMetadata,
  buildLeaseId,
  computeDefaultHeavyBudgetSlots,
  normalizeBudgetKey,
  parseActiveBudgets,
  resolveBudgetEnvKey,
  resolveHostBudgetConfig,
  resolveHostBudgetRoot,
  withHostJobBudget,
};
