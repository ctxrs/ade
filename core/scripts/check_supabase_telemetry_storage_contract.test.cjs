const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const {
  checkTelemetryStorageContract,
  detectTelemetryStoragePath,
  hasEventIdIndexContract,
  hasEventIdUniqueContract,
} = require("./check_supabase_telemetry_storage_contract.cjs");

function withFixture({ migrations, telemetrySource }, callback) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-telemetry-storage-contract-"));
  try {
    const migrationsDir = path.join(dir, "migrations");
    fs.mkdirSync(migrationsDir);
    for (const [filename, sql] of Object.entries(migrations)) {
      fs.writeFileSync(path.join(migrationsDir, filename), sql);
    }
    const telemetryPath = path.join(dir, "index.ts");
    fs.writeFileSync(telemetryPath, telemetrySource);
    callback({ migrationsDir, telemetryPath });
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
}

const indexedReadTelemetry = `
const eventIds = ingestPlan.rows.map((row) => row.event_id);
const { data: existingRows } = await client
  .from("telemetry_event")
  .select("event_id")
  .in("event_id", eventIds);
`;

const validIndexMigration = `
create index if not exists telemetry_event_event_id_idx
  on public.telemetry_event (event_id)
  where event_id is not null;
`;

const validUniqueMigration = `
create unique index if not exists telemetry_event_event_id_uidx
  on public.telemetry_event (event_id);
`;

test("detects the current indexed event-id read path", () => {
  assert.equal(detectTelemetryStoragePath(indexedReadTelemetry), "indexed_event_id_read");
});

test("accepts indexed event-id reads with the required migration", () => {
  withFixture(
    {
      migrations: {
        "20260516000100_telemetry_event_lookup_indexes.sql": validIndexMigration,
      },
      telemetrySource: indexedReadTelemetry,
    },
    ({ migrationsDir, telemetryPath }) => {
      assert.deepEqual(checkTelemetryStorageContract({ migrationsDir, telemetryPath }), {
        storagePath: "indexed_event_id_read",
      });
    },
  );
});

test("rejects indexed event-id reads when the migration is missing", () => {
  withFixture(
    {
      migrations: {
        "20260516000100_other.sql": "select 1;",
      },
      telemetrySource: indexedReadTelemetry,
    },
    ({ migrationsDir, telemetryPath }) => {
      assert.throws(
        () => checkTelemetryStorageContract({ migrationsDir, telemetryPath }),
        /telemetry_event_event_id_idx/,
      );
    },
  );
});

test("rejects the wrong index name, table, or column", () => {
  assert.equal(
    hasEventIdIndexContract([
      "create index telemetry_event_event_id_wrong_idx on public.telemetry_event (event_id) where event_id is not null;",
      "create index telemetry_event_event_id_idx on public.download_event (event_id) where event_id is not null;",
      "create index telemetry_event_event_id_idx on public.telemetry_event (event_name) where event_id is not null;",
    ]),
    false,
  );
});

test("detects unique event-id storage contracts", () => {
  assert.equal(hasEventIdUniqueContract([validUniqueMigration]), true);
  assert.equal(
    hasEventIdUniqueContract([
      "alter table public.telemetry_event add constraint telemetry_event_event_id_key unique (event_id);",
    ]),
    true,
  );
  assert.equal(
    hasEventIdUniqueContract([
      "create unique index telemetry_event_event_id_uidx on public.telemetry_event (event_name);",
    ]),
    false,
  );
});

test("accepts recognized unique event-id upsert storage", () => {
  withFixture(
    {
      migrations: {
        "20260519000100_telemetry_event_id_unique.sql": validUniqueMigration,
      },
      telemetrySource: `
await client
  .from("telemetry_event")
  .upsert(rows, { onConflict: "event_id", ignoreDuplicates: true });
`,
    },
    ({ migrationsDir, telemetryPath }) => {
      assert.deepEqual(checkTelemetryStorageContract({ migrationsDir, telemetryPath }), {
        storagePath: "unique_event_id_upsert",
      });
    },
  );
});

test("accepts recognized telemetry RPC storage", () => {
  withFixture(
    {
      migrations: {
        "20260519000100_telemetry_event_id_unique.sql": validUniqueMigration,
      },
      telemetrySource: `await client.rpc("insert_telemetry_events", { rows });`,
    },
    ({ migrationsDir, telemetryPath }) => {
      assert.deepEqual(checkTelemetryStorageContract({ migrationsDir, telemetryPath }), {
        storagePath: "telemetry_rpc",
      });
    },
  );
});

test("rejects unique event-id upsert storage without a unique contract", () => {
  withFixture(
    {
      migrations: {
        "20260516000100_telemetry_event_lookup_indexes.sql": validIndexMigration,
      },
      telemetrySource: `
await client
  .from("telemetry_event")
  .upsert(rows, { onConflict: "event_id", ignoreDuplicates: true });
`,
    },
    ({ migrationsDir, telemetryPath }) => {
      assert.throws(
        () => checkTelemetryStorageContract({ migrationsDir, telemetryPath }),
        /unique event_id index or constraint/,
      );
    },
  );
});

test("fails closed for unrecognized helper-indirection storage", () => {
  withFixture(
    {
      migrations: {
        "20260516000100_telemetry_event_lookup_indexes.sql": validIndexMigration,
      },
      telemetrySource: `await insertTelemetryRows(client, ingestPlan.rows);`,
    },
    ({ migrationsDir, telemetryPath }) => {
      assert.throws(
        () => checkTelemetryStorageContract({ migrationsDir, telemetryPath }),
        /unrecognized telemetry storage path/,
      );
    },
  );
});
