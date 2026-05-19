const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const { checkMigrationFiles } = require("./check_supabase_migrations.cjs");

function withTempMigrations(files, callback) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "ctx-supabase-migrations-"));
  try {
    for (const file of files) {
      fs.writeFileSync(path.join(dir, file), "select 1;\n");
    }
    callback(dir);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
}

test("accepts unique Supabase migration versions", () => {
  withTempMigrations(
    [
      "20260430000100_team_enterprise_launch_readiness.sql",
      "20260430000200_workos_enterprise_identity_foundation.sql",
      "20260430000300_llm_token_relay.sql",
    ],
    (dir) => {
      assert.deepEqual(checkMigrationFiles(dir), [
        "20260430000100_team_enterprise_launch_readiness.sql",
        "20260430000200_workos_enterprise_identity_foundation.sql",
        "20260430000300_llm_token_relay.sql",
      ]);
    },
  );
});

test("rejects duplicate Supabase migration versions", () => {
  withTempMigrations(
    [
      "20260428000100_llm_token_relay.sql",
      "20260428000100_team_enterprise_commercial_context.sql",
    ],
    (dir) => {
      assert.throws(
        () => checkMigrationFiles(dir),
        /duplicate migration version 20260428000100/,
      );
    },
  );
});

test("rejects malformed Supabase migration filenames", () => {
  withTempMigrations(["20260430_bad.sql"], (dir) => {
    assert.throws(
      () => checkMigrationFiles(dir),
      /bad migration filename 20260430_bad\.sql/,
    );
  });
});
