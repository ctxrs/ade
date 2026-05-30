CREATE TABLE IF NOT EXISTS ctx.ctx_users (
  id uuid PRIMARY KEY,
  primary_email text,
  display_name text,
  status text NOT NULL DEFAULT 'active'
    CHECK (status IN ('active', 'disabled', 'deleted')),
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX IF NOT EXISTS ctx_users_primary_email_lower_uidx
  ON ctx.ctx_users (lower(primary_email))
  WHERE primary_email IS NOT NULL;

CREATE TABLE IF NOT EXISTS ctx.ctx_accounts (
  id uuid PRIMARY KEY,
  ctx_user_id uuid NOT NULL REFERENCES ctx.ctx_users (id),
  account_handle text,
  status text NOT NULL DEFAULT 'active'
    CHECK (status IN ('active', 'suspended', 'deleted')),
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX IF NOT EXISTS ctx_accounts_ctx_user_id_uidx
  ON ctx.ctx_accounts (ctx_user_id);

CREATE UNIQUE INDEX IF NOT EXISTS ctx_accounts_account_handle_uidx
  ON ctx.ctx_accounts (account_handle)
  WHERE account_handle IS NOT NULL;

CREATE TABLE IF NOT EXISTS ctx.ctx_orgs (
  id uuid PRIMARY KEY,
  slug text NOT NULL,
  display_name text NOT NULL,
  status text NOT NULL DEFAULT 'active'
    CHECK (status IN ('active', 'suspended', 'deleted')),
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX IF NOT EXISTS ctx_orgs_slug_uidx
  ON ctx.ctx_orgs (slug);

CREATE TABLE IF NOT EXISTS ctx.ctx_memberships (
  id uuid PRIMARY KEY,
  ctx_org_id uuid NOT NULL REFERENCES ctx.ctx_orgs (id),
  ctx_user_id uuid NOT NULL REFERENCES ctx.ctx_users (id),
  role text NOT NULL CHECK (role IN ('owner', 'admin', 'billing_admin', 'member')),
  status text NOT NULL DEFAULT 'active'
    CHECK (status IN ('active', 'invited', 'suspended', 'removed')),
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX IF NOT EXISTS ctx_memberships_org_user_uidx
  ON ctx.ctx_memberships (ctx_org_id, ctx_user_id);

CREATE INDEX IF NOT EXISTS ctx_memberships_user_idx
  ON ctx.ctx_memberships (ctx_user_id, status);

CREATE TABLE IF NOT EXISTS ctx.billing_subjects (
  id uuid PRIMARY KEY,
  subject_kind text NOT NULL CHECK (subject_kind IN ('personal', 'org')),
  ctx_account_id uuid REFERENCES ctx.ctx_accounts (id),
  ctx_org_id uuid REFERENCES ctx.ctx_orgs (id),
  status text NOT NULL DEFAULT 'active'
    CHECK (status IN ('active', 'past_due', 'suspended', 'closed')),
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  CHECK (
    (subject_kind = 'personal' AND ctx_account_id IS NOT NULL AND ctx_org_id IS NULL)
    OR (subject_kind = 'org' AND ctx_org_id IS NOT NULL AND ctx_account_id IS NULL)
  )
);

CREATE UNIQUE INDEX IF NOT EXISTS billing_subjects_account_uidx
  ON ctx.billing_subjects (ctx_account_id)
  WHERE ctx_account_id IS NOT NULL;

CREATE UNIQUE INDEX IF NOT EXISTS billing_subjects_org_uidx
  ON ctx.billing_subjects (ctx_org_id)
  WHERE ctx_org_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS ctx.billing_entitlements (
  id uuid PRIMARY KEY,
  billing_subject_id uuid NOT NULL REFERENCES ctx.billing_subjects (id),
  entitlement_key text NOT NULL,
  entitlement_value jsonb NOT NULL DEFAULT '{}'::jsonb,
  source text NOT NULL CHECK (source IN ('stripe', 'workos', 'manual', 'relay')),
  version integer NOT NULL DEFAULT 1 CHECK (version > 0),
  active boolean NOT NULL DEFAULT true,
  valid_from timestamptz NOT NULL DEFAULT now(),
  valid_until timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  CHECK (valid_until IS NULL OR valid_until > valid_from)
);

CREATE INDEX IF NOT EXISTS billing_entitlements_subject_key_idx
  ON ctx.billing_entitlements (billing_subject_id, entitlement_key, active);

CREATE TABLE IF NOT EXISTS ctx.billing_spend_limits (
  id uuid PRIMARY KEY,
  billing_subject_id uuid NOT NULL REFERENCES ctx.billing_subjects (id),
  limit_scope text NOT NULL CHECK (limit_scope IN ('org', 'user', 'request', 'route', 'model')),
  ctx_user_id uuid REFERENCES ctx.ctx_users (id),
  route_id text,
  provider_id text,
  model_id text,
  limit_cents integer NOT NULL CHECK (limit_cents >= 0),
  period text NOT NULL CHECK (period IN ('request', 'day', 'month', 'lifetime')),
  active boolean NOT NULL DEFAULT true,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS billing_spend_limits_subject_scope_idx
  ON ctx.billing_spend_limits (billing_subject_id, limit_scope, active);

CREATE TABLE IF NOT EXISTS ctx.external_identity_links (
  id uuid PRIMARY KEY,
  provider text NOT NULL CHECK (provider IN ('workos', 'stripe')),
  external_id text NOT NULL,
  ctx_user_id uuid REFERENCES ctx.ctx_users (id),
  ctx_account_id uuid REFERENCES ctx.ctx_accounts (id),
  ctx_org_id uuid REFERENCES ctx.ctx_orgs (id),
  created_at timestamptz NOT NULL DEFAULT now(),
  CHECK (
    ((ctx_user_id IS NOT NULL)::integer
      + (ctx_account_id IS NOT NULL)::integer
      + (ctx_org_id IS NOT NULL)::integer) = 1
  )
);

CREATE UNIQUE INDEX IF NOT EXISTS external_identity_links_provider_external_uidx
  ON ctx.external_identity_links (provider, external_id);

CREATE TABLE IF NOT EXISTS ctx.stripe_webhook_events (
  event_id text PRIMARY KEY,
  event_type text NOT NULL,
  received_at timestamptz NOT NULL DEFAULT now(),
  processed_at timestamptz,
  payload_digest text NOT NULL,
  status text NOT NULL DEFAULT 'received'
    CHECK (status IN ('received', 'processed', 'failed'))
);

CREATE INDEX IF NOT EXISTS stripe_webhook_events_received_at_idx
  ON ctx.stripe_webhook_events (received_at, status);

CREATE TABLE IF NOT EXISTS ctx.workos_webhook_events (
  event_id text PRIMARY KEY,
  event_type text NOT NULL,
  received_at timestamptz NOT NULL DEFAULT now(),
  processed_at timestamptz,
  payload_digest text NOT NULL,
  status text NOT NULL DEFAULT 'received'
    CHECK (status IN ('received', 'processed', 'failed'))
);

CREATE INDEX IF NOT EXISTS workos_webhook_events_received_at_idx
  ON ctx.workos_webhook_events (received_at, status);

GRANT SELECT, INSERT, UPDATE ON
  ctx.ctx_users,
  ctx.ctx_accounts,
  ctx.ctx_orgs,
  ctx.ctx_memberships,
  ctx.billing_subjects,
  ctx.billing_entitlements,
  ctx.billing_spend_limits,
  ctx.external_identity_links
TO ctx_control_plane;

GRANT SELECT, INSERT, UPDATE ON
  ctx.stripe_webhook_events,
  ctx.billing_subjects,
  ctx.billing_entitlements,
  ctx.external_identity_links
TO ctx_stripe_webhook;

GRANT SELECT, INSERT, UPDATE ON
  ctx.workos_webhook_events,
  ctx.ctx_users,
  ctx.ctx_accounts,
  ctx.ctx_orgs,
  ctx.ctx_memberships,
  ctx.external_identity_links
TO ctx_workos_webhook;

GRANT SELECT ON
  ctx.ctx_users,
  ctx.ctx_accounts,
  ctx.ctx_orgs,
  ctx.ctx_memberships,
  ctx.billing_subjects,
  ctx.billing_entitlements,
  ctx.billing_spend_limits,
  ctx.external_identity_links,
  ctx.stripe_webhook_events,
  ctx.workos_webhook_events
TO ctx_analytics_readonly;

GRANT SELECT ON
  ctx.ctx_users,
  ctx.ctx_accounts,
  ctx.ctx_orgs,
  ctx.ctx_memberships,
  ctx.billing_subjects,
  ctx.billing_entitlements
TO ctx_mobile_tunnel;

GRANT SELECT ON
  ctx.ctx_users,
  ctx.ctx_accounts,
  ctx.ctx_orgs,
  ctx.ctx_memberships,
  ctx.billing_subjects,
  ctx.billing_entitlements,
  ctx.billing_spend_limits
TO ctx_relay_authority;
