-- Effective traffic classification for analytics.
-- Raw telemetry rows stay immutable. Corrections live in an auditable override
-- table and product analytics views resolve the effective class at query time.

create table if not exists ctx.telemetry_traffic_class_overrides (
  override_key text primary key,
  event_id text,
  origin_install_id_hash text,
  broker_install_id_hash text,
  app_version text,
  traffic_class text not null check (traffic_class in ('user', 'synthetic', 'internal', 'load_test', 'ci')),
  reason text not null check (length(trim(reason)) > 0),
  metadata jsonb not null default '{}'::jsonb,
  created_at timestamptz not null default now(),
  expires_at timestamptz,
  check (
    num_nonnulls(event_id, origin_install_id_hash, broker_install_id_hash) >= 1
  ),
  check (
    event_id is null or length(trim(event_id)) > 0
  ),
  check (
    origin_install_id_hash is null or origin_install_id_hash ~ '^[a-f0-9]{64}$'
  ),
  check (
    broker_install_id_hash is null or broker_install_id_hash ~ '^[a-f0-9]{64}$'
  ),
  check (
    app_version is null or length(trim(app_version)) > 0
  )
);

create index if not exists telemetry_traffic_class_overrides_event_idx
  on ctx.telemetry_traffic_class_overrides (event_id)
  where event_id is not null;

create index if not exists telemetry_traffic_class_overrides_origin_install_idx
  on ctx.telemetry_traffic_class_overrides (origin_install_id_hash)
  where origin_install_id_hash is not null;

create index if not exists telemetry_traffic_class_overrides_broker_install_idx
  on ctx.telemetry_traffic_class_overrides (broker_install_id_hash)
  where broker_install_id_hash is not null;


create or replace view ctx.telemetry_event_effective
with (security_invoker = true) as
select
  event.event_id,
  event.install_id_hash,
  event.broker_install_id_hash,
  event.origin_install_id_hash,
  event.occurred_at,
  event.event_name,
  event.event_version,
  event.plane,
  event.broker_runtime,
  event.origin_runtime,
  event.source,
  event.analytics_environment,
  coalesce(traffic_override.traffic_class, event.traffic_class) as traffic_class,
  event.app_version,
  event.os,
  event.arch,
  event.surface,
  event.env_target,
  event.provider_id,
  event.model_id,
  event.duration_ms,
  event.duration_bucket,
  event.status,
  event.success,
  event.session_root_kind,
  event.properties,
  event.traffic_class as raw_traffic_class,
  traffic_override.override_key as traffic_class_override_key,
  traffic_override.reason as traffic_class_override_reason,
  traffic_override.created_at as traffic_class_override_created_at
from ctx.telemetry_event event
left join lateral (
  select override.*
  from ctx.telemetry_traffic_class_overrides override
  where (override.expires_at is null or override.expires_at > now())
    and (override.event_id is null or override.event_id = event.event_id)
    and (
      override.origin_install_id_hash is null
      or override.origin_install_id_hash = event.origin_install_id_hash
    )
    and (
      override.broker_install_id_hash is null
      or override.broker_install_id_hash = event.broker_install_id_hash
    )
    and (override.app_version is null or override.app_version = event.app_version)
  order by
    (
      case when override.event_id is not null then 8 else 0 end
      + case when override.origin_install_id_hash is not null then 4 else 0 end
      + case when override.broker_install_id_hash is not null then 2 else 0 end
      + case when override.app_version is not null then 1 else 0 end
    ) desc,
    override.created_at desc,
    override.override_key
  limit 1
) traffic_override on true;

create or replace view ctx.analytics_real_product_events
with (security_invoker = true) as
select
  event_id as id,
  event_id,
  occurred_at as ts,
  occurred_at,
  event_name,
  event_version,
  plane,
  origin_install_id_hash,
  broker_install_id_hash,
  origin_runtime,
  surface,
  app_version,
  os,
  arch,
  provider_id,
  model_id,
  env_target,
  duration_ms,
  duration_bucket,
  status,
  success,
  session_root_kind,
  source,
  analytics_environment,
  traffic_class,
  properties,
  coalesce(broker_install_id_hash, origin_install_id_hash) as machine_install_id_hash
from ctx.telemetry_event_effective
where analytics_environment is not null
  and traffic_class = 'user'
  and (
    (origin_runtime = 'desktop' and surface = 'desktop')
    or (
      origin_runtime = 'daemon'
      and broker_runtime = 'daemon'
      and broker_install_id_hash is not null
      and event_name in ('session_started', 'session_completed', 'provider_call')
    )
  )
  and plane = 'product'
  and origin_install_id_hash is not null
  and event_name <> 'analytics_pipeline_smoke'
  and app_version is not null
  and app_version !~ '^0\.0\.0'
  and coalesce(provider_id, '') <> 'fake'
  and coalesce(model_id, '') <> 'fake-model'
  and (properties ->> 'smoke_run_id') is null;

grant select on table ctx.telemetry_traffic_class_overrides to ctx_analytics_readonly;
grant select, insert, update, delete on table ctx.telemetry_traffic_class_overrides to ctx_control_plane;
grant select on table ctx.telemetry_event_effective to ctx_analytics_readonly;
grant select on table ctx.telemetry_event_effective to ctx_control_plane;
grant select on table ctx.analytics_real_product_events to ctx_analytics_readonly;
