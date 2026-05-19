#!/usr/bin/env node

const fs = require("node:fs");
const path = require("node:path");

const TELEMETRY_EVENT_TABLE = "telemetry_event";
const TELEMETRY_EVENT_ID_INDEX = "telemetry_event_event_id_idx";

function repoRoot() {
  return path.join(__dirname, "..", "..");
}

function normalizeText(value) {
  return String(value).replace(/\s+/g, " ");
}

function normalizeSql(value) {
  return String(value)
    .replace(/--.*$/gm, " ")
    .replace(/\/\*[\s\S]*?\*\//g, " ")
    .replace(/\s+/g, " ")
    .trim()
    .toLowerCase();
}

function detectTelemetryStoragePath(telemetrySource) {
  const source = normalizeText(telemetrySource);
  const readsEventIds = new RegExp(
    String.raw`\.from\(\s*["']${TELEMETRY_EVENT_TABLE}["']\s*\)[\s\S]*?\.select\(\s*["'][^"']*\bevent_id\b[^"']*["']\s*\)[\s\S]*?\.in\(\s*["']event_id["']`,
    "u",
  ).test(source);
  if (readsEventIds) return "indexed_event_id_read";

  const usesEventIdUpsert = new RegExp(
    String.raw`\.from\(\s*["']${TELEMETRY_EVENT_TABLE}["']\s*\)[\s\S]*?\.upsert\(`,
    "u",
  ).test(source) && /onConflict\s*:\s*["']event_id["']/u.test(source);
  if (usesEventIdUpsert) return "unique_event_id_upsert";

  const usesTelemetryRpc = /\.rpc\(\s*["'](?:insert|record|upsert)_telemetry_events["']/u.test(source);
  if (usesTelemetryRpc) return "telemetry_rpc";

  return "unrecognized";
}

function readSqlFiles(migrationsDir) {
  if (!fs.existsSync(migrationsDir)) {
    throw new Error(`missing Supabase migrations directory: ${migrationsDir}`);
  }
  return fs
    .readdirSync(migrationsDir)
    .filter((file) => file.endsWith(".sql"))
    .sort()
    .map((file) => fs.readFileSync(path.join(migrationsDir, file), "utf8"));
}

function hasEventIdIndexContract(sqlTexts) {
  const statements = sqlTexts.flatMap((sql) => sql.split(";"));
  return statements.some((statement) => {
    const normalized = normalizeSql(statement);
    if (!normalized.includes(` ${TELEMETRY_EVENT_ID_INDEX} `)) return false;
    if (!/\bcreate\s+(?:unique\s+)?index\b/u.test(normalized)) return false;
    if (!normalized.includes(" on public.telemetry_event ")) return false;
    if (!/\(\s*event_id\s*\)/u.test(normalized)) return false;

    const isUnique = /\bcreate\s+unique\s+index\b/u.test(normalized);
    const hasNotNullPredicate = /\bwhere\b.*\bevent_id\s+is\s+not\s+null\b/u.test(normalized);
    return isUnique || hasNotNullPredicate;
  });
}

function hasEventIdUniqueContract(sqlTexts) {
  const statements = sqlTexts.flatMap((sql) => sql.split(";"));
  return statements.some((statement) => {
    const normalized = normalizeSql(statement);
    const createsUniqueIndex = /\bcreate\s+unique\s+index\b/u.test(normalized)
      && normalized.includes(" on public.telemetry_event ")
      && /\(\s*event_id\s*\)/u.test(normalized);
    const createsUniqueConstraint = /\balter\s+table\s+public\.telemetry_event\b/u.test(normalized)
      && /\badd\s+constraint\b/u.test(normalized)
      && /\bunique\s*\(\s*event_id\s*\)/u.test(normalized);
    return createsUniqueIndex || createsUniqueConstraint;
  });
}

function checkTelemetryStorageContract({
  migrationsDir = path.join(repoRoot(), "supabase", "migrations"),
  telemetryPath = path.join(repoRoot(), "supabase", "functions", "telemetry", "index.ts"),
} = {}) {
  if (!fs.existsSync(telemetryPath)) {
    throw new Error(`missing telemetry function: ${telemetryPath}`);
  }

  const telemetrySource = fs.readFileSync(telemetryPath, "utf8");
  const storagePath = detectTelemetryStoragePath(telemetrySource);
  const sqlTexts = readSqlFiles(migrationsDir);
  if (storagePath === "unique_event_id_upsert" || storagePath === "telemetry_rpc") {
    if (!hasEventIdUniqueContract(sqlTexts)) {
      throw new Error(
        "telemetry unique upsert/RPC storage requires a unique event_id index or constraint on public.telemetry_event",
      );
    }
    return { storagePath };
  }
  if (storagePath !== "indexed_event_id_read") {
    throw new Error(
      "unrecognized telemetry storage path; update check_supabase_telemetry_storage_contract.cjs with the new indexed-read or unique-upsert/RPC invariant",
    );
  }

  if (!hasEventIdIndexContract(sqlTexts)) {
    throw new Error(
      `telemetry event-id idempotency lookup requires ${TELEMETRY_EVENT_ID_INDEX} on public.telemetry_event(event_id)`,
    );
  }
  return { storagePath };
}

function main() {
  const args = process.argv.slice(2);
  const telemetryPathIndex = args.indexOf("--telemetry-path");
  const migrationsDirIndex = args.indexOf("--migrations-dir");
  const options = {};
  if (telemetryPathIndex !== -1) {
    options.telemetryPath = path.resolve(args[telemetryPathIndex + 1] || "");
  }
  if (migrationsDirIndex !== -1) {
    options.migrationsDir = path.resolve(args[migrationsDirIndex + 1] || "");
  }

  try {
    const result = checkTelemetryStorageContract(options);
    console.log(`supabase telemetry storage contract ok (${result.storagePath})`);
  } catch (error) {
    console.error(`error: ${error.message}`);
    process.exitCode = 1;
  }
}

if (require.main === module) {
  main();
}

module.exports = {
  checkTelemetryStorageContract,
  detectTelemetryStoragePath,
  hasEventIdIndexContract,
  hasEventIdUniqueContract,
};
