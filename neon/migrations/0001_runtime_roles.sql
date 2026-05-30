CREATE SCHEMA IF NOT EXISTS ctx;

COMMENT ON SCHEMA ctx IS 'Canonical ctx cloud schema for Neon-backed telemetry, control plane, mobile tunnel, and relay ledger data.';

CREATE TABLE IF NOT EXISTS ctx.neon_migration_surface_ledger (
  surface_key text PRIMARY KEY,
  description text NOT NULL,
  owner_role text NOT NULL,
  created_at timestamptz NOT NULL DEFAULT now()
);

COMMENT ON TABLE ctx.neon_migration_surface_ledger IS 'Static ledger of ctx production data surfaces covered by canonical Neon migrations.';

INSERT INTO ctx.neon_migration_surface_ledger (surface_key, description, owner_role)
VALUES
  ('telemetry', 'Product telemetry event ingestion and analytics-read models.', 'ctx_telemetry_ingest'),
  ('control_plane', 'Account, organization, membership, billing subject, and webhook foundations.', 'ctx_control_plane'),
  ('mobile_tunnel', 'Managed mobile tunnel grants, sessions, revocations, and state events.', 'ctx_mobile_tunnel'),
  ('relay_ledger', 'LLM relay routing, pricing, credit, usage, request state, and audit ledgers.', 'ctx_relay_authority')
ON CONFLICT (surface_key) DO UPDATE
SET description = EXCLUDED.description,
    owner_role = EXCLUDED.owner_role;

-- neon-role: ctx_migration
DO $$
BEGIN
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'ctx_migration') THEN
    CREATE ROLE ctx_migration LOGIN NOINHERIT;
  END IF;
END
$$;
COMMENT ON ROLE ctx_migration IS 'ctx scoped Neon role: applies canonical migrations and owns schema changes; credentials are provisioned through Infisical.';

-- neon-role: ctx_telemetry_ingest
DO $$
BEGIN
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'ctx_telemetry_ingest') THEN
    CREATE ROLE ctx_telemetry_ingest LOGIN NOINHERIT;
  END IF;
END
$$;
COMMENT ON ROLE ctx_telemetry_ingest IS 'ctx scoped Neon role: writes product telemetry events only.';

-- neon-role: ctx_analytics_readonly
DO $$
BEGIN
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'ctx_analytics_readonly') THEN
    CREATE ROLE ctx_analytics_readonly LOGIN NOINHERIT;
  END IF;
END
$$;
COMMENT ON ROLE ctx_analytics_readonly IS 'ctx scoped Neon role: reads analytics and operational reporting tables without mutation privileges.';

-- neon-role: ctx_control_plane
DO $$
BEGIN
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'ctx_control_plane') THEN
    CREATE ROLE ctx_control_plane LOGIN NOINHERIT;
  END IF;
END
$$;
COMMENT ON ROLE ctx_control_plane IS 'ctx scoped Neon role: manages ctx-owned account, organization, membership, billing, route, and entitlement state.';

-- neon-role: ctx_stripe_webhook
DO $$
BEGIN
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'ctx_stripe_webhook') THEN
    CREATE ROLE ctx_stripe_webhook LOGIN NOINHERIT;
  END IF;
END
$$;
COMMENT ON ROLE ctx_stripe_webhook IS 'ctx scoped Neon role: records Stripe webhook deliveries and maps billing facts into ctx-owned records.';

-- neon-role: ctx_workos_webhook
DO $$
BEGIN
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'ctx_workos_webhook') THEN
    CREATE ROLE ctx_workos_webhook LOGIN NOINHERIT;
  END IF;
END
$$;
COMMENT ON ROLE ctx_workos_webhook IS 'ctx scoped Neon role: records WorkOS webhook deliveries and maps identity facts into ctx-owned records.';

-- neon-role: ctx_mobile_tunnel
DO $$
BEGIN
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'ctx_mobile_tunnel') THEN
    CREATE ROLE ctx_mobile_tunnel LOGIN NOINHERIT;
  END IF;
END
$$;
COMMENT ON ROLE ctx_mobile_tunnel IS 'ctx scoped Neon role: verifies and records managed mobile tunnel grant and session state.';

-- neon-role: ctx_relay_authority
DO $$
BEGIN
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'ctx_relay_authority') THEN
    CREATE ROLE ctx_relay_authority LOGIN NOINHERIT;
  END IF;
END
$$;
COMMENT ON ROLE ctx_relay_authority IS 'ctx scoped Neon role: owns relay grant verification, budget reservation, and append-only usage ledger writes.';

GRANT USAGE, CREATE ON SCHEMA ctx TO ctx_migration;
GRANT ALL PRIVILEGES ON ALL TABLES IN SCHEMA ctx TO ctx_migration;
GRANT ALL PRIVILEGES ON ALL SEQUENCES IN SCHEMA ctx TO ctx_migration;
ALTER DEFAULT PRIVILEGES IN SCHEMA ctx GRANT ALL PRIVILEGES ON TABLES TO ctx_migration;
ALTER DEFAULT PRIVILEGES IN SCHEMA ctx GRANT ALL PRIVILEGES ON SEQUENCES TO ctx_migration;

GRANT USAGE ON SCHEMA ctx TO
  ctx_telemetry_ingest,
  ctx_analytics_readonly,
  ctx_control_plane,
  ctx_stripe_webhook,
  ctx_workos_webhook,
  ctx_mobile_tunnel,
  ctx_relay_authority;

GRANT SELECT ON ctx.neon_migration_surface_ledger TO
  ctx_analytics_readonly,
  ctx_control_plane,
  ctx_mobile_tunnel,
  ctx_relay_authority;
