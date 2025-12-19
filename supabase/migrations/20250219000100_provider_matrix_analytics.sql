-- Provider matrix request telemetry (privacy-friendly; minimal v1).

create table if not exists public.provider_matrix_event (
  id uuid primary key default gen_random_uuid(),
  ts timestamptz not null default now(),
  channel text not null,
  context_version text,
  platform text,
  result text not null,
  user_agent text,
  ip_hash text,
  country text
);

create index if not exists provider_matrix_event_ts_idx on public.provider_matrix_event (ts desc);

alter table public.provider_matrix_event enable row level security;

revoke all on table public.provider_matrix_event from anon, authenticated, public;

grant all on table public.provider_matrix_event to service_role;

drop policy if exists service_role_access on public.provider_matrix_event;
create policy service_role_access on public.provider_matrix_event
  for all
  to service_role
  using (true)
  with check (true);
