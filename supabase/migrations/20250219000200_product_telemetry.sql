-- Product telemetry tables (anonymous, opt-in, metadata-only).

create table if not exists public.telemetry_event (
  id uuid primary key default gen_random_uuid(),
  ts timestamptz not null default now(),
  occurred_at timestamptz,
  event_name text not null,
  install_id_hash text,
  app_version text,
  os text,
  arch text,
  provider_id text,
  model_id text,
  env_target text,
  duration_ms bigint,
  status text,
  success boolean
);

create index if not exists telemetry_event_ts_idx on public.telemetry_event (ts desc);
create index if not exists telemetry_event_name_idx on public.telemetry_event (event_name);
create index if not exists telemetry_event_provider_idx on public.telemetry_event (provider_id);

-- Security: telemetry tables are service-role only.
alter table public.telemetry_event enable row level security;

revoke all on table public.telemetry_event from anon, authenticated, public;
grant all on table public.telemetry_event to service_role;

drop policy if exists service_role_access on public.telemetry_event;
create policy service_role_access on public.telemetry_event
  for all
  to service_role
  using (true)
  with check (true);
