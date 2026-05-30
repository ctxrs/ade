const fs = require("node:fs");
const path = require("node:path");

const MIGRATION_FILE_RE = /^(\d{4})_[a-z0-9_]+\.sql$/;

const REQUIRED_ROLES = Object.freeze([
  "ctx_migration",
  "ctx_telemetry_ingest",
  "ctx_analytics_readonly",
  "ctx_control_plane",
  "ctx_stripe_webhook",
  "ctx_workos_webhook",
  "ctx_mobile_tunnel",
  "ctx_relay_authority",
]);

const REQUIRED_TABLES = Object.freeze([
  "ctx.neon_migration_surface_ledger",
  "ctx.ctx_users",
  "ctx.ctx_accounts",
  "ctx.ctx_orgs",
  "ctx.ctx_memberships",
  "ctx.billing_subjects",
  "ctx.billing_entitlements",
  "ctx.billing_spend_limits",
  "ctx.external_identity_links",
  "ctx.stripe_webhook_events",
  "ctx.workos_webhook_events",
  "ctx.telemetry_event",
  "ctx.mobile_tunnel_grants",
  "ctx.mobile_tunnel_sessions",
  "ctx.mobile_tunnel_state_events",
  "ctx.mobile_tunnel_revocations",
  "ctx.route_configs",
  "ctx.route_policy_versions",
  "ctx.pricing_catalog_versions",
  "ctx.model_prices",
  "ctx.credit_grants",
  "ctx.usage_reservations",
  "ctx.usage_reservation_credit_allocations",
  "ctx.credit_ledger_events",
  "ctx.usage_ledger_events",
  "ctx.request_state_events",
  "ctx.audit_events",
  "ctx.relay_grant_jti_consumptions",
  "ctx.provider_invoice_lines",
]);

const REQUIRED_INDEXES = Object.freeze([
  "ctx_users_primary_email_lower_uidx",
  "ctx_accounts_ctx_user_id_uidx",
  "ctx_orgs_slug_uidx",
  "ctx_memberships_org_user_uidx",
  "ctx_memberships_user_idx",
  "billing_subjects_account_uidx",
  "billing_subjects_org_uidx",
  "billing_entitlements_subject_key_idx",
  "billing_spend_limits_subject_scope_idx",
  "external_identity_links_provider_external_uidx",
  "stripe_webhook_events_received_at_idx",
  "workos_webhook_events_received_at_idx",
  "telemetry_event_event_id_idx",
  "telemetry_event_event_name_ts_idx",
  "telemetry_event_install_ts_idx",
  "telemetry_event_env_ts_idx",
  "mobile_tunnel_grants_jti_uidx",
  "mobile_tunnel_grants_subject_expiry_idx",
  "mobile_tunnel_sessions_tunnel_uidx",
  "mobile_tunnel_sessions_status_seen_idx",
  "mobile_tunnel_state_events_session_observed_idx",
  "mobile_tunnel_revocations_jti_idx",
  "route_configs_route_uidx",
  "route_configs_subject_status_idx",
  "route_policy_versions_active_idx",
  "model_prices_version_provider_model_uidx",
  "credit_grants_spendable_idx",
  "usage_reservations_request_uidx",
  "usage_reservations_jti_uidx",
  "usage_reservations_billing_status_idx",
  "usage_reservation_allocations_reservation_idx",
  "usage_reservation_allocations_grant_idx",
  "credit_ledger_events_request_idx",
  "usage_ledger_events_request_idx",
  "request_state_events_request_idx",
  "audit_events_subject_time_idx",
  "relay_grant_jti_consumptions_request_uidx",
  "provider_invoice_lines_request_idx",
]);

const REQUIRED_SQL_PATTERNS = Object.freeze([
  {
    label: "telemetry ingest role can return idempotent inserted event ids",
    pattern: /\bgrant\s+insert\s*,\s*select\s*\(\s*event_id\s*\)\s+on\s+ctx\s*\.\s*telemetry_event\s+to\s+ctx_telemetry_ingest\b/i,
  },
]);

const LEGACY_LEDGER_NAME = "supa" + "base_migrations";

const FORBIDDEN_PATTERNS = Object.freeze([
  { label: "auth.uid()", pattern: /\bauth\s*\.\s*uid\s*\(/i },
  { label: "auth.users", pattern: /\bauth\s*\.\s*users\b/i },
  { label: "service_role", pattern: /\bservice_role\b/i },
  { label: "authenticated", pattern: /\bauthenticated\b/i },
  { label: "anon", pattern: /\banon\b/i },
  { label: "legacy migration ledger", pattern: new RegExp(`\\b${LEGACY_LEDGER_NAME}\\b`, "i") },
  { label: "row level security", pattern: /\brow\s+level\s+security\b/i },
  { label: "create policy", pattern: /\bcreate\s+policy\b/i },
  { label: "alter policy", pattern: /\balter\s+policy\b/i },
]);

function defaultMigrationsDir() {
  return path.resolve(__dirname, "..", "..", "neon", "migrations");
}

function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function stripSqlComments(sql) {
  return sql
    .replace(/\/\*[\s\S]*?\*\//g, " ")
    .replace(/--.*$/gm, " ");
}

function normalizeSql(sql) {
  return stripSqlComments(sql).replace(/\s+/g, " ").trim();
}

function listMigrationFiles(migrationsDir) {
  if (!fs.existsSync(migrationsDir)) {
    return {
      files: [],
      errors: [`missing migrations directory: ${migrationsDir}`],
    };
  }

  const entries = fs.readdirSync(migrationsDir, { withFileTypes: true });
  const files = entries
    .filter((entry) => entry.isFile() && entry.name.endsWith(".sql"))
    .map((entry) => entry.name)
    .sort();

  return { files, errors: [] };
}

function checkFilenameOrdering(files) {
  const errors = [];
  const seenPrefixes = new Map();

  files.forEach((file) => {
    const match = MIGRATION_FILE_RE.exec(file);
    if (!match) {
      errors.push(`${file}: migration filenames must match NNNN_lower_snake_case.sql`);
      return;
    }

    const prefix = match[1];
    const existing = seenPrefixes.get(prefix);
    if (existing) {
      errors.push(`${file}: duplicate migration prefix ${prefix}; already used by ${existing}`);
    } else {
      seenPrefixes.set(prefix, file);
    }
  });

  const orderedPrefixes = files
    .map((file) => MIGRATION_FILE_RE.exec(file))
    .filter(Boolean)
    .map((match) => Number.parseInt(match[1], 10));

  orderedPrefixes.forEach((prefix, index) => {
    const expected = index + 1;
    if (prefix !== expected) {
      errors.push(`migration prefix ${String(prefix).padStart(4, "0")} must be contiguous; expected ${String(expected).padStart(4, "0")}`);
    }
  });

  return errors;
}

function readMigrations(migrationsDir, files) {
  return files.map((file) => ({
    file,
    sql: fs.readFileSync(path.join(migrationsDir, file), "utf8"),
  }));
}

function checkForbiddenConstructs(migrations) {
  const errors = [];
  migrations.forEach(({ file, sql }) => {
    FORBIDDEN_PATTERNS.forEach(({ label, pattern }) => {
      if (pattern.test(sql)) {
        errors.push(`${file}: forbidden construct found: ${label}`);
      }
    });
  });
  return errors;
}

function hasCreateTable(normalizedSql, tableName) {
  const [schemaName, relationName] = tableName.split(".");
  const pattern = new RegExp(
    `\\bcreate\\s+table\\s+(?:if\\s+not\\s+exists\\s+)?${escapeRegExp(schemaName)}\\s*\\.\\s*${escapeRegExp(relationName)}\\b`,
    "i",
  );
  return pattern.test(normalizedSql);
}

function hasCreateIndex(normalizedSql, indexName) {
  const pattern = new RegExp(
    `\\bcreate\\s+(?:unique\\s+)?index\\s+(?:if\\s+not\\s+exists\\s+)?${escapeRegExp(indexName)}\\b`,
    "i",
  );
  return pattern.test(normalizedSql);
}

function hasRoleMarker(rawSql, roleName) {
  return rawSql.includes(`-- neon-role: ${roleName}`);
}

function hasRoleComment(normalizedSql, roleName) {
  const pattern = new RegExp(`\\bcomment\\s+on\\s+role\\s+${escapeRegExp(roleName)}\\s+is\\s+'`, "i");
  return pattern.test(normalizedSql);
}

function hasRoleGrant(normalizedSql, roleName) {
  const pattern = new RegExp(`\\bgrant\\b[^;]*\\bto\\b[^;]*\\b${escapeRegExp(roleName)}\\b`, "i");
  return pattern.test(normalizedSql);
}

function checkRoleExpectations(rawSql, normalizedSql) {
  const errors = [];

  REQUIRED_ROLES.forEach((roleName) => {
    if (!hasRoleMarker(rawSql, roleName)) {
      errors.push(`missing role marker: -- neon-role: ${roleName}`);
    }
    if (!hasRoleComment(normalizedSql, roleName)) {
      errors.push(`missing COMMENT ON ROLE for ${roleName}`);
    }
    if (!hasRoleGrant(normalizedSql, roleName)) {
      errors.push(`missing GRANT for ${roleName}`);
    }
  });

  return errors;
}

function checkRequiredDomainObjects(normalizedSql) {
  const errors = [];

  REQUIRED_TABLES.forEach((tableName) => {
    if (!hasCreateTable(normalizedSql, tableName)) {
      errors.push(`missing required table: ${tableName}`);
    }
  });

  REQUIRED_INDEXES.forEach((indexName) => {
    if (!hasCreateIndex(normalizedSql, indexName)) {
      errors.push(`missing required index: ${indexName}`);
    }
  });

  REQUIRED_SQL_PATTERNS.forEach(({ label, pattern }) => {
    if (!pattern.test(normalizedSql)) {
      errors.push(`missing required SQL contract: ${label}`);
    }
  });

  return errors;
}

function checkMigrationDirectory(migrationsDir = defaultMigrationsDir()) {
  const listed = listMigrationFiles(migrationsDir);
  const errors = [...listed.errors];

  if (listed.files.length === 0) {
    errors.push(`no .sql migration files found in ${migrationsDir}`);
    return { files: listed.files, errors };
  }

  errors.push(...checkFilenameOrdering(listed.files));

  const migrations = readMigrations(migrationsDir, listed.files);
  const rawSql = migrations.map((migration) => migration.sql).join("\n\n");
  const normalizedSql = normalizeSql(rawSql);

  errors.push(...checkForbiddenConstructs(migrations));
  errors.push(...checkRoleExpectations(rawSql, normalizedSql));
  errors.push(...checkRequiredDomainObjects(normalizedSql));

  return { files: listed.files, errors };
}

function parseArgs(argv) {
  const parsed = {
    migrationsDir: defaultMigrationsDir(),
  };

  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "--migrations-dir") {
      const value = argv[index + 1];
      if (!value) {
        throw new Error("--migrations-dir requires a path");
      }
      parsed.migrationsDir = path.resolve(value);
      index += 1;
      continue;
    }
    throw new Error(`unknown argument: ${arg}`);
  }

  return parsed;
}

function main(argv) {
  let parsed;
  try {
    parsed = parseArgs(argv);
  } catch (error) {
    console.error(error.message);
    process.exitCode = 64;
    return;
  }

  const result = checkMigrationDirectory(parsed.migrationsDir);
  if (result.errors.length > 0) {
    console.error("Neon migration contract failed:");
    result.errors.forEach((error) => {
      console.error(`- ${error}`);
    });
    process.exitCode = 1;
    return;
  }

  console.log(`Neon migration contract OK: ${result.files.length} migration files checked.`);
}

if (require.main === module) {
  main(process.argv.slice(2));
}

module.exports = {
  REQUIRED_INDEXES,
  REQUIRED_SQL_PATTERNS,
  REQUIRED_ROLES,
  REQUIRED_TABLES,
  checkMigrationDirectory,
  defaultMigrationsDir,
  parseArgs,
};
