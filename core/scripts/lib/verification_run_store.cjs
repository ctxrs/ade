const crypto = require("node:crypto");
const fs = require("node:fs");
const path = require("node:path");

const { resolveCtxCacheLayout } = require("./cache_roots.cjs");
const { summarizeHostSamples } = require("./verification_host_sampler.cjs");

const VERIFICATION_RUNS_DIRNAME = "verification-runs";
const DEFAULT_MAX_RUNS = 200;
const DEFAULT_MAX_AGE_DAYS = 14;
const INDEX_FILENAME = "index.jsonl";

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

function acquireLock(lockPath, { retries = 200, retryDelayMs = 25 } = {}) {
  for (let attempt = 0; attempt < retries; attempt += 1) {
    try {
      fs.mkdirSync(lockPath);
      return () => fs.rmSync(lockPath, { recursive: true, force: true });
    } catch (error) {
      if (error?.code !== "EEXIST") {
        throw error;
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
  const normalizedSummary = {
    ...summary,
    runId: runArtifacts.runId,
    kind: runArtifacts.kind,
    entrypoint: runArtifacts.entrypoint,
    hostSamplesPath: path.relative(runArtifacts.rootDir, runArtifacts.hostSamplesPath),
    hostStats: summary.hostStats || summarizeHostSamples(runArtifacts.hostSamplesPath),
  };
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
  DEFAULT_MAX_RUNS,
  INDEX_FILENAME,
  VERIFICATION_RUNS_DIRNAME,
  acquireLock,
  createRunArtifacts,
  createRunId,
  finalizeRunArtifacts,
  pruneRunStore,
  readIndexedSummaries,
  readSummaryFile,
  rebuildIndex,
  resolveRetentionPolicy,
  resolveRunStoreRoot,
  writeJsonAtomic,
  writeTextAtomic,
};
