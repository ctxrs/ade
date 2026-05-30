CREATE TABLE IF NOT EXISTS ctx.mobile_tunnel_grants (
  grant_id uuid PRIMARY KEY,
  grant_jti text NOT NULL,
  audience text NOT NULL CHECK (audience = 'ctx-mobile-tunnel'),
  ctx_user_id uuid NOT NULL REFERENCES ctx.ctx_users (id),
  ctx_account_id uuid REFERENCES ctx.ctx_accounts (id),
  ctx_org_id uuid REFERENCES ctx.ctx_orgs (id),
  billing_subject_id uuid NOT NULL REFERENCES ctx.billing_subjects (id),
  daemon_id text NOT NULL,
  device_id text NOT NULL,
  entitlement_version text NOT NULL,
  scopes text[] NOT NULL DEFAULT ARRAY[]::text[],
  grant_digest text NOT NULL,
  issued_at timestamptz NOT NULL,
  expires_at timestamptz NOT NULL,
  revoked_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  CHECK (expires_at > issued_at)
);

CREATE UNIQUE INDEX IF NOT EXISTS mobile_tunnel_grants_jti_uidx
  ON ctx.mobile_tunnel_grants (grant_jti);

CREATE INDEX IF NOT EXISTS mobile_tunnel_grants_subject_expiry_idx
  ON ctx.mobile_tunnel_grants (billing_subject_id, ctx_user_id, expires_at);

CREATE TABLE IF NOT EXISTS ctx.mobile_tunnel_sessions (
  session_id uuid PRIMARY KEY,
  grant_id uuid NOT NULL REFERENCES ctx.mobile_tunnel_grants (grant_id),
  tunnel_id text NOT NULL,
  daemon_id text NOT NULL,
  device_id text NOT NULL,
  status text NOT NULL DEFAULT 'pending'
    CHECK (status IN ('pending', 'active', 'closed', 'revoked', 'expired')),
  opened_at timestamptz NOT NULL DEFAULT now(),
  last_seen_at timestamptz NOT NULL DEFAULT now(),
  closed_at timestamptz
);

CREATE UNIQUE INDEX IF NOT EXISTS mobile_tunnel_sessions_tunnel_uidx
  ON ctx.mobile_tunnel_sessions (tunnel_id);

CREATE INDEX IF NOT EXISTS mobile_tunnel_sessions_status_seen_idx
  ON ctx.mobile_tunnel_sessions (status, last_seen_at);

CREATE TABLE IF NOT EXISTS ctx.mobile_tunnel_state_events (
  event_id uuid PRIMARY KEY,
  session_id uuid REFERENCES ctx.mobile_tunnel_sessions (session_id),
  grant_id uuid REFERENCES ctx.mobile_tunnel_grants (grant_id),
  event_type text NOT NULL,
  observed_at timestamptz NOT NULL DEFAULT now(),
  metadata jsonb NOT NULL DEFAULT '{}'::jsonb,
  CHECK (session_id IS NOT NULL OR grant_id IS NOT NULL)
);

CREATE INDEX IF NOT EXISTS mobile_tunnel_state_events_session_observed_idx
  ON ctx.mobile_tunnel_state_events (session_id, observed_at)
  WHERE session_id IS NOT NULL;

CREATE TABLE IF NOT EXISTS ctx.mobile_tunnel_revocations (
  revocation_id uuid PRIMARY KEY,
  grant_jti text NOT NULL,
  reason text NOT NULL,
  revoked_at timestamptz NOT NULL DEFAULT now(),
  revoked_by_role text NOT NULL
);

CREATE INDEX IF NOT EXISTS mobile_tunnel_revocations_jti_idx
  ON ctx.mobile_tunnel_revocations (grant_jti, revoked_at);

GRANT SELECT ON
  ctx.mobile_tunnel_grants
TO ctx_mobile_tunnel;

GRANT SELECT, INSERT, UPDATE ON
  ctx.mobile_tunnel_sessions,
  ctx.mobile_tunnel_state_events,
  ctx.mobile_tunnel_revocations
TO ctx_mobile_tunnel;

GRANT SELECT, INSERT, UPDATE ON
  ctx.mobile_tunnel_grants,
  ctx.mobile_tunnel_sessions,
  ctx.mobile_tunnel_revocations
TO ctx_control_plane;

GRANT SELECT ON
  ctx.mobile_tunnel_grants,
  ctx.mobile_tunnel_sessions,
  ctx.mobile_tunnel_state_events,
  ctx.mobile_tunnel_revocations
TO ctx_analytics_readonly;
