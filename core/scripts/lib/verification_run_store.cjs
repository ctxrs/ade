const crypto = require("node:crypto");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const { resolveCtxCacheLayout } = require("./cache_roots.cjs");
const { summarizeHostSamples } = require("./verification_host_sampler.cjs");

const VERIFICATION_RUNS_DIRNAME = "verification-runs";
const DEFAULT_MAX_RUNS = 200;
const DEFAULT_MAX_AGE_DAYS = 14;
const DEFAULT_LOCK_STALE_AFTER_MS = 30 * 1000;
const INDEX_FILENAME = "index.jsonl";
const LOCK_OWNER_FILENAME = "owner.json";

function sleepMs(durationMs) {
  Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, durationMs);
}

function resolveRunStoreRoot({ cwd, env = process.env, layout } = {}) {
  const effectiveLayout = layout || resolveCtxCacheLayout({ cwd, env });
  return path.join(effectiveLayout.artifactsDir, VERIFICATION_RUNS_DIRNAME);
}

function createRunId() {
  return [
    new Date().toISOString().replace(/[:.]/gu, "-"),
    process.pid,
    crypto.randomBytes(4).toString("hex"),
  ].join("-");
}

function ensureDir(dirPath) {
  fs.mkdirSync(dirPath, { recursive: true });
  return dirPath;
}

function writeTextAtomic(filePath, contents) {
  ensureDir(path.dirname(filePath));
  const tempPath = `${filePath}.tmp-${process.pid}-${crypto.randomBytes(4).toString("hex")}`;
  fs.writeFileSync(tempPath, contents, "utf8");
  fs.renameSync(tempPath, filePath);
}

function writeJsonAtomic(filePath, payload) {
  writeTextAtomic(filePath, `${JSON.stringify(payload, null, 2)}\n`);
}

function createLockOwnerRecord() {
  return {
    acquiredAt: new Date().toISOString(),
    hostname: os.hostname(),
    pid: process.pid,
    token: crypto.randomBytes(8).toString("hex"),
  };
}

function readJsonIfPresent(filePath) {
  try {
    if (!fs.existsSync(filePath)) {
      return null;
    }
    return JSON.parse(fs.readFileSync(filePath, "utf8"));
  } catch {
    return null;
  }
}

function readLockOwnerRecord(lockPath) {
  return readJsonIfPresent(path.join(lockPath, LOCK_OWNER_FILENAME));
}

function writeLockOwnerRecord(lockPath, ownerRecord) {
  fs.writeFileSync(
    path.join(lockPath, LOCK_OWNER_FILENAME),
    `${JSON.stringify(ownerRecord, null, 2)}\n`,
    "utf8",
  );
}

function isLiveProcess(pid) {
  if (!Number.isInteger(pid) || pid <= 0) {
    return false;
  }
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    if (error?.code === "EPERM") {
      return true;
    }
    return false;
  }
}

function getLockAgeMs(lockPath, ownerRecord, nowMs) {
  const acquiredAtMs = Date.parse(ownerRecord?.acquiredAt || "");
  if (Number.isFinite(acquiredAtMs)) {
    return Math.max(0, nowMs - acquiredAtMs);
  }
  try {
    return Math.max(0, nowMs - fs.statSync(lockPath).mtimeMs);
  } catch {
    return 0;
  }
}

function observeLock(lockPath) {
  try {
    const stats = fs.statSync(lockPath);
    return {
      dev: stats.dev,
      ino: stats.ino,
      mtimeMs: stats.mtimeMs,
      ownerRecord: readLockOwnerRecord(lockPath),
    };
  } catch {
    return null;
  }
}

function isSameLockObservation(left, right) {
  if (left == null || right == null) {
    return false;
  }
  const leftToken = String(left.ownerRecord?.token || "");
  const rightToken = String(right.ownerRecord?.token || "");
  if (leftToken || rightToken) {
    return leftToken !== "" && leftToken === rightToken;
  }
  return left.dev === right.dev && left.ino === right.ino && left.mtimeMs === right.mtimeMs;
}

function getStaleLockObservation(lockPath, {
  nowMs = Date.now(),
  staleAfterMs = DEFAULT_LOCK_STALE_AFTER_MS,
} = {}) {
  const observation = observeLock(lockPath);
  if (observation == null) {
    return null;
  }
  const { ownerRecord } = observation;
  const sameHostOwner = ownerRecord?.hostname && ownerRecord.hostname === os.hostname();
  if (sameHostOwner && !isLiveProcess(ownerRecord.pid)) {
    return observation;
  }
  if (sameHostOwner) {
    return null;
  }
  return getLockAgeMs(lockPath, ownerRecord, nowMs) >= staleAfterMs
    ? observation
    : null;
}

function breakStaleLock(lockPath, observation) {
  if (!isSameLockObservation(observation, observeLock(lockPath))) {
    return false;
  }
  fs.rmSync(lockPath, { recursive: true, force: true });
  return true;
}

function releaseLock(lockPath, ownerRecord) {
  if (!fs.existsSync(lockPath)) {
    return;
  }
  const currentOwnerRecord = readLockOwnerRecord(lockPath);
  if (currentOwnerRecord?.token && currentOwnerRecord.token !== ownerRecord.token) {
    return;
  }
  fs.rmSync(lockPath, { recursive: true, force: true });
}

function toNonNegativeMs(value) {
  const parsed = Number(value);
  if (!Number.isFinite(parsed) || parsed <= 0) {
    return 0;
  }
  return parsed;
}

function isPresentMetricValue(value) {
  return value !== undefined && value !== null && value !== "";
}

function toOptionalNonNegativeMs(value) {
  if (!isPresentMetricValue(value)) {
    return undefined;
  }
  const parsed = Number(value);
  if (!Number.isFinite(parsed) || parsed < 0) {
    return undefined;
  }
  return parsed;
}

function toOptionalBoolean(value) {
  if (!isPresentMetricValue(value)) {
    return undefined;
  }
  if (typeof value === "boolean") {
    return value;
  }
  if (value === 0 || value === 1) {
    return Boolean(value);
  }
  const normalized = String(value).trim().toLowerCase();
  if (normalized === "true") {
    return true;
  }
  if (normalized === "false") {
    return false;
  }
  return Boolean(value);
}

function sumInstrumentedMetric(entries, readMetric, { emptyValue = undefined } = {}) {
  if (!Array.isArray(entries)) {
    return undefined;
  }
  if (entries.length === 0) {
    return emptyValue;
  }
  let total = 0;
  for (const entry of entries) {
    const value = readMetric(entry);
    if (value === undefined) {
      return undefined;
    }
    total += value;
  }
  return total;
}

function normalizeCacheHitShapeValue(value) {
  const normalized = String(value || "").trim();
  return normalized;
}

function normalizeCacheStatsStatusValue(value) {
  return String(value || "").trim();
}

function hasPositiveCacheStat(cacheStats, keys) {
  if (!cacheStats || typeof cacheStats !== "object") {
    return false;
  }
  return keys.some((key) => Number(cacheStats[key] || 0) > 0);
}

function deriveCacheHitShape(cacheStats) {
  if (!cacheStats || typeof cacheStats !== "object") {
    return "";
  }
  const actionCacheHits = Number(
    cacheStats.actionCacheHits
    || cacheStats.action_cache_hits
    || 0,
  );
  const actionCacheMisses = Number(
    cacheStats.actionCacheMisses
    || cacheStats.action_cache_misses
    || 0,
  );
  const casCacheHits = Number(
    cacheStats.casCacheHits
    || cacheStats.cas_cache_hits
    || 0,
  );
  const casCacheMisses = Number(
    cacheStats.casCacheMisses
    || cacheStats.cas_cache_misses
    || 0,
  );
  if (actionCacheHits > 0 && actionCacheMisses === 0) {
    return "action-cache-all-hit";
  }
  if (actionCacheHits > 0 && actionCacheMisses > 0) {
    return "action-cache-mixed";
  }
  if (actionCacheMisses > 0) {
    return "action-cache-miss";
  }
  if (casCacheHits > 0 && casCacheMisses === 0) {
    return "cas-cache-all-hit";
  }
  if (casCacheHits > 0 && casCacheMisses > 0) {
    return "cas-cache-mixed";
  }
  if (casCacheMisses > 0) {
    return "cas-cache-miss";
  }
  return "";
}

function hasBuildBuddyCacheEvidence(summary) {
  return Boolean(
    summary?.buildBuddyEnabled
    || String(summary?.remoteInvocationUrl || "").trim()
    || (Array.isArray(summary?.buildBuddyInvocations) && summary.buildBuddyInvocations.length > 0),
  );
}

function deriveCacheStatsStatus(summary) {
  const explicitStatus = normalizeCacheStatsStatusValue(summary?.cacheStatsStatus);
  if (explicitStatus) {
    return explicitStatus;
  }
  const cacheStats = summary?.cacheStats;
  if (cacheStats && typeof cacheStats === "object") {
    if (hasPositiveCacheStat(cacheStats, [
      "actionCacheHits",
      "actionCacheMisses",
      "action_cache_hits",
      "action_cache_misses",
      "actionsCreated",
      "actionsExecuted",
      "actions_created",
      "actions_executed",
    ])) {
      return "action-cache-stats-reported";
    }
    if (hasPositiveCacheStat(cacheStats, [
      "casCacheHits",
      "casCacheMisses",
      "cas_cache_hits",
      "cas_cache_misses",
    ])) {
      return "cas-cache-stats-reported";
    }
    return "cache-stats-empty";
  }
  const remoteExecutionMode = String(summary?.remoteExecutionMode || "").trim();
  if (hasBuildBuddyCacheEvidence(summary)) {
    return "buildbuddy-invocation-without-cache-stats";
  }
  if (remoteExecutionMode === "off") {
    return "local-cache-only";
  }
  if (remoteExecutionMode) {
    return "remote-cache-stats-unavailable";
  }
  return "cache-stats-unavailable";
}

function inferCacheHitShape(summary) {
  const explicitShape = normalizeCacheHitShapeValue(summary?.cacheHitShape);
  if (explicitShape) {
    return explicitShape;
  }
  const derivedShape = deriveCacheHitShape(summary?.cacheStats);
  if (derivedShape) {
    return derivedShape;
  }
  const remoteExecutionMode = String(summary?.remoteExecutionMode || "").trim();
  if (hasBuildBuddyCacheEvidence(summary)) {
    return "buildbuddy-cache-unreported";
  }
  if (remoteExecutionMode === "off") {
    return "local-cache-only";
  }
  if (remoteExecutionMode) {
    return "cache-evidence-unavailable";
  }
  return "cache-evidence-unavailable";
}

function deriveBazelMetrics(summary) {
  const phases = Array.isArray(summary.phases) ? summary.phases : [];
  const localPhases = phases.filter((phase) => phase?.name === "local");
  const remotePhases = phases.filter((phase) => phase?.name !== "local");
  const durationMs = summary.durationMs != null
    ? toNonNegativeMs(summary.durationMs)
    : phases.reduce((total, phase) => total + toNonNegativeMs(phase.durationMs), 0);
  const queueTimeMs = summary.queueTimeMs != null
    ? toOptionalNonNegativeMs(summary.queueTimeMs)
    : sumInstrumentedMetric(phases, (phase) => toOptionalNonNegativeMs(phase?.queueTimeMs));
  const remoteActionTimeMs = summary.remoteActionTimeMs != null
    ? toOptionalNonNegativeMs(summary.remoteActionTimeMs)
    : Array.isArray(summary.phases)
      ? sumInstrumentedMetric(
      remotePhases,
      (phase) => {
        const phaseDurationMs = toOptionalNonNegativeMs(phase?.durationMs);
        const phaseQueueTimeMs = toOptionalNonNegativeMs(phase?.queueTimeMs);
        if (phaseDurationMs === undefined || phaseQueueTimeMs === undefined) {
          return undefined;
        }
        return Math.max(0, phaseDurationMs - phaseQueueTimeMs);
      },
      { emptyValue: 0 },
    )
      : undefined;
  const explicitRunnerLocalOverheadMs = toOptionalNonNegativeMs(summary.runnerLocalOverheadMs);
  return {
    durationMs,
    queueTimeMs,
    remoteActionTimeMs,
    cacheHitShape: inferCacheHitShape(summary),
    cacheStatsStatus: deriveCacheStatsStatus(summary),
    runnerLocalOverheadMs: explicitRunnerLocalOverheadMs !== undefined
      ? explicitRunnerLocalOverheadMs
      : remoteActionTimeMs !== undefined
        ? Math.max(0, durationMs - remoteActionTimeMs)
        : undefined,
    localSpill: summary.localSpill != null
      ? toOptionalBoolean(summary.localSpill)
      : Array.isArray(summary.phases)
        ? localPhases.length > 0 && remotePhases.length > 0
        : undefined,
  };
}

function combineCacheHitShapes(entries) {
  const uniqueShapes = [...new Set(
    entries
      .map((entry) => normalizeCacheHitShapeValue(entry?.cacheHitShape))
      .filter(Boolean),
  )];
  if (uniqueShapes.length === 0) {
    return "";
  }
  if (uniqueShapes.length === 1) {
    return uniqueShapes[0];
  }
  return "mixed";
}

function combineCacheStatsStatuses(entries) {
  const uniqueStatuses = [...new Set(
    entries
      .map((entry) => normalizeCacheStatsStatusValue(entry?.cacheStatsStatus))
      .filter(Boolean),
  )];
  if (uniqueStatuses.length === 0) {
    return "";
  }
  if (uniqueStatuses.length === 1) {
    return uniqueStatuses[0];
  }
  return "mixed";
}

function normalizeChildBazelRuns(entries) {
  if (!Array.isArray(entries)) {
    return [];
  }
  return entries.map((entry) => normalizeVerificationRunSummary({
    ...entry,
    kind: entry?.kind || "bazel",
    entrypoint: entry?.entrypoint || "run_bazel_pilot",
  }));
}

function deriveRouterMetrics(summary) {
  const childBazelRuns = normalizeChildBazelRuns(summary.childBazelRuns);
  const durationMs = toNonNegativeMs(summary.durationMs);
  const queueTimeMs = summary.queueTimeMs != null
    ? toOptionalNonNegativeMs(summary.queueTimeMs)
    : sumInstrumentedMetric(
      childBazelRuns,
      (entry) => toOptionalNonNegativeMs(entry?.queueTimeMs),
    );
  const remoteActionTimeMs = summary.remoteActionTimeMs != null
    ? toOptionalNonNegativeMs(summary.remoteActionTimeMs)
    : sumInstrumentedMetric(
      childBazelRuns,
      (entry) => toOptionalNonNegativeMs(entry?.remoteActionTimeMs),
    );
  const remoteSetupMs = toOptionalNonNegativeMs(summary.remoteSetupMs);
  const childLocalOverheadMs = sumInstrumentedMetric(
    childBazelRuns,
    (entry) => toOptionalNonNegativeMs(entry?.runnerLocalOverheadMs),
  );
  const explicitRunnerLocalOverheadMs = toOptionalNonNegativeMs(summary.runnerLocalOverheadMs);
  const runnerLocalOverheadMs = explicitRunnerLocalOverheadMs !== undefined
    ? explicitRunnerLocalOverheadMs
    : remoteSetupMs !== undefined && childLocalOverheadMs !== undefined
      ? remoteSetupMs + childLocalOverheadMs
      : remoteSetupMs !== undefined && childBazelRuns.length === 0
        ? remoteSetupMs
        : undefined;
  const explicitLocalSpill = toOptionalBoolean(summary.localSpill);
  const childLocalSpillValues = childBazelRuns.map((entry) => toOptionalBoolean(entry?.localSpill));
  const localSpill = explicitLocalSpill !== undefined
    ? explicitLocalSpill
    : childLocalSpillValues.length > 0 && childLocalSpillValues.every((value) => value !== undefined)
      ? childLocalSpillValues.some(Boolean)
      : undefined;
  return {
    childBazelRuns,
    durationMs,
    queueTimeMs,
    remoteActionTimeMs,
    cacheHitShape:
      normalizeCacheHitShapeValue(summary.cacheHitShape)
      || combineCacheHitShapes(childBazelRuns),
    cacheStatsStatus:
      normalizeCacheStatsStatusValue(summary.cacheStatsStatus)
      || combineCacheStatsStatuses(childBazelRuns),
    runnerLocalOverheadMs,
    localSpill,
  };
}

function normalizeVerificationRunSummary(summary) {
  if (!summary || typeof summary !== "object") {
    return summary;
  }
  if (summary.kind === "bazel") {
    return {
      ...summary,
      ...deriveBazelMetrics(summary),
    };
  }
  if (summary.kind === "router") {
    return {
      ...summary,
      ...deriveRouterMetrics(summary),
    };
  }
  return summary;
}

function acquireLock(lockPath, {
  retries = 200,
  retryDelayMs = 25,
  staleAfterMs = DEFAULT_LOCK_STALE_AFTER_MS,
} = {}) {
  const ownerRecord = createLockOwnerRecord();
  for (let attempt = 0; attempt < retries; attempt += 1) {
    try {
      fs.mkdirSync(lockPath);
      try {
        writeLockOwnerRecord(lockPath, ownerRecord);
      } catch (error) {
        fs.rmSync(lockPath, { recursive: true, force: true });
        throw error;
      }
      return () => releaseLock(lockPath, ownerRecord);
    } catch (error) {
      if (error?.code !== "EEXIST") {
        throw error;
      }
      const staleLockObservation = getStaleLockObservation(lockPath, { staleAfterMs });
      if (staleLockObservation != null && breakStaleLock(lockPath, staleLockObservation)) {
        attempt -= 1;
        continue;
      }
      sleepMs(retryDelayMs);
    }
  }
  throw new Error(`timed out acquiring verification run-store lock: ${lockPath}`);
}

function parsePositiveInteger(value, fallback) {
  const normalized = String(value ?? "").trim();
  if (!normalized) {
    return fallback;
  }
  if (!/^\d+$/u.test(normalized)) {
    return fallback;
  }
  const parsed = Number.parseInt(normalized, 10);
  return parsed > 0 ? parsed : fallback;
}

function resolveRetentionPolicy(env = process.env) {
  return {
    maxRuns: parsePositiveInteger(env.CTX_VERIFICATION_RUN_MAX_COUNT, DEFAULT_MAX_RUNS),
    maxAgeMs: parsePositiveInteger(env.CTX_VERIFICATION_RUN_MAX_AGE_DAYS, DEFAULT_MAX_AGE_DAYS) * 24 * 60 * 60 * 1000,
  };
}

function listRunSummaryFiles(rootDir) {
  if (!fs.existsSync(rootDir)) {
    return [];
  }
  return fs.readdirSync(rootDir, { withFileTypes: true })
    .filter((entry) => entry.isDirectory())
    .map((entry) => path.join(rootDir, entry.name, "summary.json"))
    .filter((summaryPath) => fs.existsSync(summaryPath));
}

function readSummaryFile(summaryPath) {
  return JSON.parse(fs.readFileSync(summaryPath, "utf8"));
}

function summarizeForOrdering(summaryPath) {
  const summary = readSummaryFile(summaryPath);
  const sortKey = Date.parse(summary.completedAt || summary.startedAt || "") || 0;
  return {
    summary,
    summaryPath,
    sortKey,
    runDir: path.dirname(summaryPath),
  };
}

function rebuildIndex(rootDir) {
  const summaries = listRunSummaryFiles(rootDir)
    .map(summarizeForOrdering)
    .sort((left, right) => left.sortKey - right.sortKey)
    .map(({ summary }) => JSON.stringify(summary))
    .join("\n");
  writeTextAtomic(
    path.join(rootDir, INDEX_FILENAME),
    summaries.length > 0 ? `${summaries}\n` : "",
  );
}

function pruneRunStore(rootDir, { env = process.env, now = Date.now() } = {}) {
  const retention = resolveRetentionPolicy(env);
  const summaries = listRunSummaryFiles(rootDir)
    .map(summarizeForOrdering)
    .sort((left, right) => right.sortKey - left.sortKey);

  summaries.forEach(({ runDir, sortKey }, index) => {
    const tooOld = sortKey > 0 && retention.maxAgeMs > 0 && now - sortKey > retention.maxAgeMs;
    const overLimit = index >= retention.maxRuns;
    if (tooOld || overLimit) {
      fs.rmSync(runDir, { recursive: true, force: true });
    }
  });
}

function createRunArtifacts({
  cwd,
  env = process.env,
  entrypoint,
  kind,
  layout,
  runId = createRunId(),
} = {}) {
  const rootDir = resolveRunStoreRoot({ cwd, env, layout });
  const runDir = path.join(rootDir, runId);
  return {
    entrypoint,
    kind,
    rootDir,
    runDir,
    runId,
    summaryPath: path.join(runDir, "summary.json"),
    hostSamplesPath: path.join(runDir, "host-samples.jsonl"),
  };
}

function finalizeRunArtifacts(runArtifacts, summary, { env = process.env } = {}) {
  ensureDir(runArtifacts.runDir);
  const normalizedSummary = normalizeVerificationRunSummary({
    ...summary,
    runId: runArtifacts.runId,
    kind: runArtifacts.kind,
    entrypoint: runArtifacts.entrypoint,
    hostSamplesPath: path.relative(runArtifacts.rootDir, runArtifacts.hostSamplesPath),
    hostStats: summary.hostStats || summarizeHostSamples(runArtifacts.hostSamplesPath),
  });
  writeJsonAtomic(runArtifacts.summaryPath, normalizedSummary);

  const releaseLock = acquireLock(path.join(runArtifacts.rootDir, ".lock"));
  try {
    pruneRunStore(runArtifacts.rootDir, { env });
    rebuildIndex(runArtifacts.rootDir);
  } finally {
    releaseLock();
  }

  return normalizedSummary;
}

function readIndexedSummaries({ cwd, env = process.env, layout } = {}) {
  const rootDir = resolveRunStoreRoot({ cwd, env, layout });
  const indexPath = path.join(rootDir, INDEX_FILENAME);
  if (!fs.existsSync(indexPath)) {
    return [];
  }
  return fs.readFileSync(indexPath, "utf8")
    .split(/\r?\n/u)
    .map((line) => line.trim())
    .filter(Boolean)
    .map((line) => JSON.parse(line));
}

module.exports = {
  DEFAULT_MAX_AGE_DAYS,
  DEFAULT_LOCK_STALE_AFTER_MS,
  DEFAULT_MAX_RUNS,
  INDEX_FILENAME,
  LOCK_OWNER_FILENAME,
  VERIFICATION_RUNS_DIRNAME,
  acquireLock,
  breakStaleLock,
  createRunArtifacts,
  createRunId,
  deriveBazelMetrics,
  inferCacheHitShape,
  deriveRouterMetrics,
  finalizeRunArtifacts,
  getStaleLockObservation,
  normalizeVerificationRunSummary,
  pruneRunStore,
  readIndexedSummaries,
  readSummaryFile,
  rebuildIndex,
  resolveRetentionPolicy,
  resolveRunStoreRoot,
  writeJsonAtomic,
  writeTextAtomic,
};
