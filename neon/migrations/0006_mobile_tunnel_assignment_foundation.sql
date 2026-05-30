CREATE TABLE IF NOT EXISTS ctx.mobile_tunnel_relay_node (
  relay_id text PRIMARY KEY,
  region text NOT NULL,
  public_base_url text NOT NULL,
  internal_base_url text NOT NULL,
  status text NOT NULL DEFAULT 'active'
    CHECK (status IN ('active', 'disabled', 'draining')),
  active_tunnel_count integer NOT NULL DEFAULT 0 CHECK (active_tunnel_count >= 0),
  max_active_tunnels integer NOT NULL CHECK (max_active_tunnels > 0),
  last_heartbeat_at timestamptz,
  heartbeat_expires_at timestamptz,
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS mobile_tunnel_relay_node_region_health_idx
  ON ctx.mobile_tunnel_relay_node (region, status, heartbeat_expires_at, active_tunnel_count);

CREATE TABLE IF NOT EXISTS ctx.mobile_tunnel (
  tunnel_id text PRIMARY KEY,
  user_id text NOT NULL,
  billing_subject_id text,
  relay_id text NOT NULL REFERENCES ctx.mobile_tunnel_relay_node (relay_id),
  public_base_url text NOT NULL,
  status text NOT NULL DEFAULT 'active'
    CHECK (status IN ('active', 'revoked', 'disabled', 'expired')),
  created_at timestamptz NOT NULL DEFAULT now(),
  updated_at timestamptz NOT NULL DEFAULT now(),
  disabled_at timestamptz,
  last_connected_at timestamptz,
  last_accessed_at timestamptz
);

CREATE INDEX IF NOT EXISTS mobile_tunnel_user_active_idx
  ON ctx.mobile_tunnel (user_id, status, disabled_at, created_at);

CREATE TABLE IF NOT EXISTS ctx.mobile_tunnel_event (
  event_id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  tunnel_id text,
  relay_id text,
  user_id text,
  event_type text NOT NULL,
  metadata jsonb NOT NULL DEFAULT '{}'::jsonb,
  observed_at timestamptz NOT NULL DEFAULT now(),
  created_at timestamptz NOT NULL DEFAULT now(),
  CHECK (tunnel_id IS NOT NULL OR relay_id IS NOT NULL OR user_id IS NOT NULL)
);

CREATE INDEX IF NOT EXISTS mobile_tunnel_event_tunnel_observed_idx
  ON ctx.mobile_tunnel_event (tunnel_id, observed_at)
  WHERE tunnel_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS mobile_tunnel_event_user_observed_idx
  ON ctx.mobile_tunnel_event (user_id, observed_at)
  WHERE user_id IS NOT NULL;

GRANT SELECT, INSERT, UPDATE ON
  ctx.mobile_tunnel_relay_node,
  ctx.mobile_tunnel,
  ctx.mobile_tunnel_event
TO ctx_mobile_tunnel;

GRANT DELETE ON
  ctx.mobile_tunnel,
  ctx.mobile_tunnel_event
TO ctx_mobile_tunnel;

GRANT SELECT ON
  ctx.mobile_tunnel_relay_node,
  ctx.mobile_tunnel,
  ctx.mobile_tunnel_event
TO ctx_analytics_readonly;
