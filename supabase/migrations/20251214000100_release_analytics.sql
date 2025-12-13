-- Release + update analytics tables (privacy-friendly; minimal v1).

create table if not exists public.download_event (
  id uuid primary key default gen_random_uuid(),
  ts timestamptz not null default now(),
  channel text not null,
  version text not null,
  platform text,
  artifact text,
  status text not null,
  url_path text not null,
  referrer text,
  user_agent text,
  ip_hash text,
  country text
);

create index if not exists download_event_ts_idx on public.download_event (ts desc);
create index if not exists download_event_version_idx on public.download_event (version);

create table if not exists public.update_check_event (
  id uuid primary key default gen_random_uuid(),
  ts timestamptz not null default now(),
  channel text not null,
  current_version text,
  platform text,
  result text not null,
  latest_version text,
  install_id_hash text,
  user_agent text,
  ip_hash text,
  country text
);

create index if not exists update_check_event_ts_idx on public.update_check_event (ts desc);

create table if not exists public.download_daily_agg (
  day date not null,
  channel text not null,
  version text not null,
  platform text,
  artifact text,
  count bigint not null default 0,
  unique_estimate bigint,
  primary key (day, channel, version, platform, artifact)
);

-- Security: these are telemetry tables; lock them down.
alter table public.download_event enable row level security;
alter table public.update_check_event enable row level security;
alter table public.download_daily_agg enable row level security;

revoke all on table public.download_event from anon, authenticated, public;
revoke all on table public.update_check_event from anon, authenticated, public;
revoke all on table public.download_daily_agg from anon, authenticated, public;

grant all on table public.download_event to service_role;
grant all on table public.update_check_event to service_role;
grant all on table public.download_daily_agg to service_role;

drop policy if exists service_role_access on public.download_event;
create policy service_role_access on public.download_event
  for all
  to service_role
  using (true)
  with check (true);

drop policy if exists service_role_access on public.update_check_event;
create policy service_role_access on public.update_check_event
  for all
  to service_role
  using (true)
  with check (true);

drop policy if exists service_role_access on public.download_daily_agg;
create policy service_role_access on public.download_daily_agg
  for all
  to service_role
  using (true)
  with check (true);
