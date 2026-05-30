CREATE TABLE IF NOT EXISTS ctx.telemetry_event (
  event_id text PRIMARY KEY,
  install_id_hash text,
  broker_install_id_hash text,
  origin_install_id_hash text NOT NULL,
  occurred_at timestamptz NOT NULL,
  event_name text NOT NULL,
  event_version integer NOT NULL CHECK (event_version > 0),
  plane text NOT NULL CHECK (plane IN ('product', 'incident')),
  broker_runtime text NOT NULL,
  origin_runtime text NOT NULL,
  source text,
  analytics_environment text,
  traffic_class text NOT NULL CHECK (traffic_class IN ('user', 'synthetic', 'internal', 'load_test', 'ci')),
  app_version text NOT NULL,
  os text NOT NULL,
  arch text NOT NULL,
  surface text,
  env_target text,
  provider_id text,
  model_id text,
  duration_ms integer CHECK (duration_ms IS NULL OR duration_ms >= 0),
  duration_bucket text,
  status text,
  success boolean,
  session_root_kind text,
  properties jsonb NOT NULL DEFAULT '{}'::jsonb
);

CREATE UNIQUE INDEX IF NOT EXISTS telemetry_event_event_id_idx
  ON ctx.telemetry_event (event_id);

CREATE INDEX IF NOT EXISTS telemetry_event_event_name_ts_idx
  ON ctx.telemetry_event (event_name, occurred_at);

CREATE INDEX IF NOT EXISTS telemetry_event_install_ts_idx
  ON ctx.telemetry_event (broker_install_id_hash, origin_install_id_hash, occurred_at);

CREATE INDEX IF NOT EXISTS telemetry_event_env_ts_idx
  ON ctx.telemetry_event (analytics_environment, traffic_class, occurred_at);

CREATE INDEX IF NOT EXISTS telemetry_event_properties_gin_idx
  ON ctx.telemetry_event USING gin (properties);

GRANT INSERT ON ctx.telemetry_event TO ctx_telemetry_ingest;
GRANT SELECT ON ctx.telemetry_event TO ctx_analytics_readonly;
GRANT SELECT ON ctx.telemetry_event TO ctx_control_plane;
