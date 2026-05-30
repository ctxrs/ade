const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const {
  checkMigrationDirectory,
  defaultMigrationsDir,
} = require("./check_neon_migrations.cjs");

function makeTempMigrations() {
  const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-neon-migrations-"));
  const tempMigrations = path.join(tempRoot, "migrations");
  fs.cpSync(defaultMigrationsDir(), tempMigrations, { recursive: true });
  return { tempRoot, tempMigrations };
}

function withTempMigrations(callback) {
  const fixture = makeTempMigrations();
  try {
    callback(fixture.tempMigrations);
  } finally {
    fs.rmSync(fixture.tempRoot, { recursive: true, force: true });
  }
}

function appendToMigration(migrationsDir, fileName, text) {
  fs.appendFileSync(path.join(migrationsDir, fileName), text);
}

function replaceInMigration(migrationsDir, fileName, fromText, toText) {
  const filePath = path.join(migrationsDir, fileName);
  const before = fs.readFileSync(filePath, "utf8");
  assert.match(before, new RegExp(fromText.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")));
  fs.writeFileSync(filePath, before.replace(fromText, toText));
}

test("repository Neon migrations satisfy the static contract", () => {
  const result = checkMigrationDirectory();
  assert.deepEqual(result.errors, []);
});

test("duplicate filename prefixes fail ordering validation", () => {
  withTempMigrations((migrationsDir) => {
    fs.writeFileSync(path.join(migrationsDir, "0001_duplicate.sql"), "SELECT 1;\n");

    const result = checkMigrationDirectory(migrationsDir);

    assert.match(result.errors.join("\n"), /duplicate migration prefix 0001/);
  });
});

test("forbidden Supabase-only SQL constructs fail validation", () => {
  withTempMigrations((migrationsDir) => {
    appendToMigration(migrationsDir, "0003_telemetry_event_foundation.sql", "\nSELECT auth.uid();\n");

    const result = checkMigrationDirectory(migrationsDir);

    assert.match(result.errors.join("\n"), /forbidden construct found: auth\.uid\(\)/);
  });
});

test("missing runtime role marker fails validation", () => {
  withTempMigrations((migrationsDir) => {
    replaceInMigration(
      migrationsDir,
      "0001_runtime_roles.sql",
      "-- neon-role: ctx_mobile_tunnel",
      "-- omitted-role: ctx_mobile_tunnel",
    );

    const result = checkMigrationDirectory(migrationsDir);

    assert.match(result.errors.join("\n"), /missing role marker: -- neon-role: ctx_mobile_tunnel/);
  });
});

test("missing required domain table fails validation", () => {
  withTempMigrations((migrationsDir) => {
    replaceInMigration(
      migrationsDir,
      "0003_telemetry_event_foundation.sql",
      "CREATE TABLE IF NOT EXISTS ctx.telemetry_event",
      "CREATE TABLE IF NOT EXISTS ctx.telemetry_event_missing",
    );

    const result = checkMigrationDirectory(migrationsDir);

    assert.match(result.errors.join("\n"), /missing required table: ctx\.telemetry_event/);
  });
});

test("missing required domain index fails validation", () => {
  withTempMigrations((migrationsDir) => {
    replaceInMigration(
      migrationsDir,
      "0005_relay_ledger_foundation.sql",
      "CREATE INDEX IF NOT EXISTS usage_ledger_events_request_idx",
      "CREATE INDEX IF NOT EXISTS usage_ledger_events_request_missing_idx",
    );

    const result = checkMigrationDirectory(migrationsDir);

    assert.match(result.errors.join("\n"), /missing required index: usage_ledger_events_request_idx/);
  });
});

test("missing telemetry ingest returning grant fails validation", () => {
  withTempMigrations((migrationsDir) => {
    replaceInMigration(
      migrationsDir,
      "0003_telemetry_event_foundation.sql",
      "GRANT INSERT, SELECT (event_id) ON ctx.telemetry_event TO ctx_telemetry_ingest;",
      "GRANT INSERT ON ctx.telemetry_event TO ctx_telemetry_ingest;",
    );

    const result = checkMigrationDirectory(migrationsDir);

    assert.match(result.errors.join("\n"), /missing required SQL contract: telemetry ingest role can return idempotent inserted event ids/);
  });
});

test("missing relay spend limit runtime columns fail validation", () => {
  withTempMigrations((migrationsDir) => {
    replaceInMigration(
      migrationsDir,
      "0008_relay_spend_limits_and_mobile_grant_hardening.sql",
      "ADD COLUMN IF NOT EXISTS hard_limit_cents integer",
      "ADD COLUMN IF NOT EXISTS soft_limit_cents integer",
    );

    const result = checkMigrationDirectory(migrationsDir);

    assert.match(result.errors.join("\n"), /missing required SQL contract: relay spend limit schema exposes runtime enforcement columns/);
  });
});

test("missing mobile grant runtime revoke fails validation", () => {
  withTempMigrations((migrationsDir) => {
    replaceInMigration(
      migrationsDir,
      "0008_relay_spend_limits_and_mobile_grant_hardening.sql",
      "REVOKE INSERT, UPDATE ON ctx.mobile_tunnel_grants FROM ctx_mobile_tunnel;",
      "GRANT SELECT ON ctx.mobile_tunnel_grants TO ctx_mobile_tunnel;",
    );

    const result = checkMigrationDirectory(migrationsDir);

    assert.match(result.errors.join("\n"), /missing required SQL contract: mobile tunnel runtime role cannot mutate grant rows/);
  });
});

test("missing mobile tunnel binding index fails validation", () => {
  withTempMigrations((migrationsDir) => {
    replaceInMigration(
      migrationsDir,
      "0009_mobile_tunnel_binding_hardening.sql",
      "CREATE INDEX IF NOT EXISTS mobile_tunnel_binding_active_idx",
      "CREATE INDEX IF NOT EXISTS mobile_tunnel_binding_active_missing_idx",
    );

    const result = checkMigrationDirectory(migrationsDir);

    assert.match(result.errors.join("\n"), /missing required index: mobile_tunnel_binding_active_idx/);
  });
});

test("missing relay pricing precision columns fail validation", () => {
  withTempMigrations((migrationsDir) => {
    replaceInMigration(
      migrationsDir,
      "0005_relay_ledger_foundation.sql",
      "input_microusd_per_1k_tokens",
      "input_microusd_per_token_legacy_only",
    );
    replaceInMigration(
      migrationsDir,
      "0010_relay_pricing_precision.sql",
      "input_microusd_per_1k_tokens",
      "input_microusd_per_token_legacy_only",
    );

    const result = checkMigrationDirectory(migrationsDir);

    assert.match(result.errors.join("\n"), /missing required SQL contract: relay pricing preserves per-1k token precision/);
  });
});
