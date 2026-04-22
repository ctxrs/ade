alter table public.telemetry_event
  add column if not exists event_id text,
  add column if not exists event_version integer,
  add column if not exists plane text,
  add column if not exists broker_install_id_hash text,
  add column if not exists broker_runtime text,
  add column if not exists origin_install_id_hash text,
  add column if not exists origin_runtime text,
  add column if not exists surface text,
  add column if not exists source text,
  add column if not exists duration_bucket text,
  add column if not exists session_root_kind text,
  add column if not exists properties jsonb not null default '{}'::jsonb;

create index if not exists telemetry_event_plane_idx on public.telemetry_event (plane);
create index if not exists telemetry_event_origin_install_idx on public.telemetry_event (origin_install_id_hash);
create index if not exists telemetry_event_broker_install_idx on public.telemetry_event (broker_install_id_hash);
create index if not exists telemetry_event_surface_idx on public.telemetry_event (surface);
