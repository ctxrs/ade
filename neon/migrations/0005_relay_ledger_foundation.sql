CREATE TABLE IF NOT EXISTS ctx.route_configs (
  id uuid PRIMARY KEY,
  route_id text NOT NULL,
  billing_subject_id uuid NOT NULL REFERENCES ctx.billing_subjects (id),
  route_type text NOT NULL CHECK (route_type IN ('ctx_managed', 'user_managed', 'customer_gateway')),
  credential_owner text NOT NULL CHECK (credential_owner IN ('ctx', 'user', 'customer')),
  auth_method text NOT NULL CHECK (auth_method IN ('ctx_provider_key', 'oauth', 'api_key', 'gateway_token', 'mtls')),
  governance_level text NOT NULL CHECK (governance_level IN ('hard_enforced', 'policy_enforced_local', 'receipt_enforced')),
  status text NOT NULL CHECK (status IN ('enabled', 'disabled', 'planned')),
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX IF NOT EXISTS route_configs_route_uidx
  ON ctx.route_configs (route_id);

CREATE INDEX IF NOT EXISTS route_configs_subject_status_idx
  ON ctx.route_configs (billing_subject_id, status);

CREATE TABLE IF NOT EXISTS ctx.route_policy_versions (
  policy_version text PRIMARY KEY,
  billing_subject_id uuid REFERENCES ctx.billing_subjects (id),
  route_id text REFERENCES ctx.route_configs (route_id),
  policy_document jsonb NOT NULL,
  status text NOT NULL CHECK (status IN ('draft', 'active', 'superseded', 'revoked')),
  effective_at timestamptz NOT NULL DEFAULT now(),
  created_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS route_policy_versions_active_idx
  ON ctx.route_policy_versions (billing_subject_id, route_id, effective_at)
  WHERE status = 'active';

CREATE TABLE IF NOT EXISTS ctx.pricing_catalog_versions (
  pricing_version text PRIMARY KEY,
  status text NOT NULL CHECK (status IN ('draft', 'active', 'superseded', 'revoked')),
  currency text NOT NULL DEFAULT 'USD',
  effective_at timestamptz NOT NULL DEFAULT now(),
  created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS ctx.model_prices (
  id uuid PRIMARY KEY,
  pricing_version text NOT NULL REFERENCES ctx.pricing_catalog_versions (pricing_version),
  provider_id text NOT NULL,
  model_id text NOT NULL,
  input_microusd_per_token integer NOT NULL CHECK (input_microusd_per_token >= 0),
  output_microusd_per_token integer NOT NULL CHECK (output_microusd_per_token >= 0),
  input_microusd_per_1k_tokens bigint NOT NULL DEFAULT 0 CHECK (input_microusd_per_1k_tokens >= 0),
  output_microusd_per_1k_tokens bigint NOT NULL DEFAULT 0 CHECK (output_microusd_per_1k_tokens >= 0),
  request_overhead_cents integer NOT NULL DEFAULT 0 CHECK (request_overhead_cents >= 0),
  valid_from timestamptz NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX IF NOT EXISTS model_prices_version_provider_model_uidx
  ON ctx.model_prices (pricing_version, provider_id, model_id);

CREATE TABLE IF NOT EXISTS ctx.credit_grants (
  id uuid PRIMARY KEY,
  billing_subject_id uuid NOT NULL REFERENCES ctx.billing_subjects (id),
  credit_source text NOT NULL
    CHECK (credit_source IN ('promo', 'trial', 'subscription', 'enterprise_commit', 'prepaid_top_up', 'manual_adjustment')),
  original_cents integer NOT NULL CHECK (original_cents >= 0),
  remaining_cents integer NOT NULL CHECK (remaining_cents >= 0),
  expires_at timestamptz,
  active boolean NOT NULL DEFAULT true,
  created_at timestamptz NOT NULL DEFAULT now(),
  CHECK (remaining_cents <= original_cents)
);

CREATE INDEX IF NOT EXISTS credit_grants_spendable_idx
  ON ctx.credit_grants (billing_subject_id, active, expires_at, credit_source)
  WHERE active = true AND remaining_cents > 0;

CREATE TABLE IF NOT EXISTS ctx.usage_reservations (
  reservation_id uuid PRIMARY KEY,
  request_id text NOT NULL,
  run_grant_jti text NOT NULL,
  delegation_jti text NOT NULL,
  billing_subject_id uuid NOT NULL REFERENCES ctx.billing_subjects (id),
  ctx_user_id uuid NOT NULL REFERENCES ctx.ctx_users (id),
  ctx_account_id uuid REFERENCES ctx.ctx_accounts (id),
  ctx_org_id uuid REFERENCES ctx.ctx_orgs (id),
  route_id text NOT NULL REFERENCES ctx.route_configs (route_id),
  provider_id text NOT NULL,
  model_id text NOT NULL,
  policy_version text NOT NULL REFERENCES ctx.route_policy_versions (policy_version),
  pricing_version text NOT NULL REFERENCES ctx.pricing_catalog_versions (pricing_version),
  max_estimated_cents integer NOT NULL CHECK (max_estimated_cents >= 0),
  reserved_cents integer NOT NULL CHECK (reserved_cents >= 0),
  actual_billable_cents integer CHECK (actual_billable_cents IS NULL OR actual_billable_cents >= 0),
  status text NOT NULL CHECK (
    status IN (
      'rejected_preflight',
      'reserved',
      'provider_started',
      'stream_completed',
      'stream_broken_usage_unknown',
      'finalized',
      'voided',
      'reconciled'
    )
  ),
  provider_request_id text,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  finalized_at timestamptz
);

CREATE UNIQUE INDEX IF NOT EXISTS usage_reservations_request_uidx
  ON ctx.usage_reservations (request_id);

CREATE UNIQUE INDEX IF NOT EXISTS usage_reservations_jti_uidx
  ON ctx.usage_reservations (run_grant_jti);

CREATE INDEX IF NOT EXISTS usage_reservations_billing_status_idx
  ON ctx.usage_reservations (billing_subject_id, status, created_at);

CREATE INDEX IF NOT EXISTS usage_reservations_provider_request_idx
  ON ctx.usage_reservations (provider_request_id)
  WHERE provider_request_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS ctx.usage_reservation_credit_allocations (
  allocation_id uuid PRIMARY KEY,
  reservation_id uuid NOT NULL REFERENCES ctx.usage_reservations (reservation_id),
  credit_grant_id uuid NOT NULL REFERENCES ctx.credit_grants (id),
  reserved_cents integer NOT NULL CHECK (reserved_cents >= 0),
  debited_cents integer NOT NULL DEFAULT 0 CHECK (debited_cents >= 0),
  released_cents integer NOT NULL DEFAULT 0 CHECK (released_cents >= 0),
  created_at timestamptz NOT NULL DEFAULT now(),
  CHECK (debited_cents + released_cents <= reserved_cents)
);

CREATE INDEX IF NOT EXISTS usage_reservation_allocations_reservation_idx
  ON ctx.usage_reservation_credit_allocations (reservation_id);

CREATE INDEX IF NOT EXISTS usage_reservation_allocations_grant_idx
  ON ctx.usage_reservation_credit_allocations (credit_grant_id);

CREATE TABLE IF NOT EXISTS ctx.credit_ledger_events (
  event_id uuid PRIMARY KEY,
  credit_grant_id uuid REFERENCES ctx.credit_grants (id),
  billing_subject_id uuid NOT NULL REFERENCES ctx.billing_subjects (id),
  event_type text NOT NULL
    CHECK (event_type IN ('grant_created', 'reserved', 'released', 'debited', 'expired', 'voided', 'reconciled')),
  amount_cents integer NOT NULL,
  request_id text,
  usage_reservation_id uuid REFERENCES ctx.usage_reservations (reservation_id),
  created_at timestamptz NOT NULL DEFAULT now(),
  metadata jsonb NOT NULL DEFAULT '{}'::jsonb
);

CREATE INDEX IF NOT EXISTS credit_ledger_events_request_idx
  ON ctx.credit_ledger_events (request_id, created_at)
  WHERE request_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS ctx.usage_ledger_events (
  event_id uuid PRIMARY KEY,
  reservation_id uuid REFERENCES ctx.usage_reservations (reservation_id),
  request_id text NOT NULL,
  event_type text NOT NULL
    CHECK (
      event_type IN (
        'rejected_preflight',
        'reserved',
        'provider_started',
        'stream_completed',
        'stream_broken_usage_unknown',
        'finalized',
        'voided',
        'reconciled'
      )
    ),
  amount_cents integer,
  input_tokens integer CHECK (input_tokens IS NULL OR input_tokens >= 0),
  output_tokens integer CHECK (output_tokens IS NULL OR output_tokens >= 0),
  created_at timestamptz NOT NULL DEFAULT now(),
  metadata jsonb NOT NULL DEFAULT '{}'::jsonb
);

CREATE INDEX IF NOT EXISTS usage_ledger_events_request_idx
  ON ctx.usage_ledger_events (request_id, created_at);

CREATE TABLE IF NOT EXISTS ctx.request_state_events (
  event_id uuid PRIMARY KEY,
  request_id text NOT NULL,
  reservation_id uuid REFERENCES ctx.usage_reservations (reservation_id),
  from_state text,
  to_state text NOT NULL,
  occurred_at timestamptz NOT NULL DEFAULT now(),
  reason text,
  metadata jsonb NOT NULL DEFAULT '{}'::jsonb
);

CREATE INDEX IF NOT EXISTS request_state_events_request_idx
  ON ctx.request_state_events (request_id, occurred_at);

CREATE TABLE IF NOT EXISTS ctx.audit_events (
  event_id uuid PRIMARY KEY,
  audit_subject_kind text NOT NULL,
  audit_subject_id text NOT NULL,
  actor_user_id uuid REFERENCES ctx.ctx_users (id),
  action text NOT NULL,
  occurred_at timestamptz NOT NULL DEFAULT now(),
  request_id text,
  metadata jsonb NOT NULL DEFAULT '{}'::jsonb
);

CREATE INDEX IF NOT EXISTS audit_events_subject_time_idx
  ON ctx.audit_events (audit_subject_kind, audit_subject_id, occurred_at);

CREATE TABLE IF NOT EXISTS ctx.relay_grant_jti_consumptions (
  jti text PRIMARY KEY,
  request_id text NOT NULL,
  delegation_jti text NOT NULL,
  reservation_id uuid REFERENCES ctx.usage_reservations (reservation_id),
  consumed_at timestamptz NOT NULL DEFAULT now(),
  provider_started_at timestamptz,
  status text NOT NULL CHECK (status IN ('reserved', 'provider_started', 'voided'))
);

CREATE UNIQUE INDEX IF NOT EXISTS relay_grant_jti_consumptions_request_uidx
  ON ctx.relay_grant_jti_consumptions (request_id);

CREATE TABLE IF NOT EXISTS ctx.provider_invoice_lines (
  line_id uuid PRIMARY KEY,
  provider_id text NOT NULL,
  provider_invoice_id text NOT NULL,
  provider_request_id text,
  billing_subject_id uuid REFERENCES ctx.billing_subjects (id),
  request_id text,
  input_tokens integer CHECK (input_tokens IS NULL OR input_tokens >= 0),
  output_tokens integer CHECK (output_tokens IS NULL OR output_tokens >= 0),
  provider_cost_microusd integer NOT NULL CHECK (provider_cost_microusd >= 0),
  observed_at timestamptz NOT NULL DEFAULT now(),
  metadata jsonb NOT NULL DEFAULT '{}'::jsonb
);

CREATE INDEX IF NOT EXISTS provider_invoice_lines_request_idx
  ON ctx.provider_invoice_lines (provider_id, provider_request_id, request_id);

GRANT SELECT, INSERT, UPDATE ON
  ctx.route_configs,
  ctx.route_policy_versions,
  ctx.pricing_catalog_versions,
  ctx.model_prices,
  ctx.credit_grants
TO ctx_control_plane;

GRANT SELECT, INSERT, UPDATE ON
  ctx.credit_grants,
  ctx.credit_ledger_events
TO ctx_stripe_webhook;

GRANT SELECT ON
  ctx.route_configs,
  ctx.route_policy_versions,
  ctx.pricing_catalog_versions,
  ctx.model_prices,
  ctx.credit_grants,
  ctx.usage_reservations,
  ctx.usage_reservation_credit_allocations,
  ctx.credit_ledger_events,
  ctx.usage_ledger_events,
  ctx.request_state_events,
  ctx.audit_events,
  ctx.relay_grant_jti_consumptions,
  ctx.provider_invoice_lines
TO ctx_analytics_readonly;

GRANT SELECT ON
  ctx.route_configs,
  ctx.route_policy_versions,
  ctx.pricing_catalog_versions,
  ctx.model_prices,
  ctx.billing_subjects,
  ctx.billing_entitlements,
  ctx.billing_spend_limits
TO ctx_relay_authority;

GRANT SELECT, UPDATE ON ctx.credit_grants TO ctx_relay_authority;

GRANT SELECT, INSERT, UPDATE ON
  ctx.usage_reservations,
  ctx.usage_reservation_credit_allocations,
  ctx.relay_grant_jti_consumptions
TO ctx_relay_authority;

GRANT SELECT, INSERT ON
  ctx.credit_ledger_events,
  ctx.usage_ledger_events,
  ctx.request_state_events,
  ctx.audit_events,
  ctx.provider_invoice_lines
TO ctx_relay_authority;
