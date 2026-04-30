-- Mobile tunnel control-plane state.
--
-- This schema intentionally lives in Supabase Postgres, not on the Hetzner
-- node. Control-plane, router, and relay use the restricted
-- `ctx_tunnel_service` role so relay assignment remains durable across nodes
-- and can move behind a load balancer without a data migration.

create extension if not exists pgcrypto;

create or replace function public.set_row_updated_at()
returns trigger
language plpgsql
as $$
begin
  new.updated_at = now();
  return new;
end;
$$;

do $$
begin
  if not exists (select 1 from pg_roles where rolname = 'ctx_tunnel_service') then
    create role ctx_tunnel_service login;
  end if;
end $$;

create table if not exists public.mobile_tunnel_relay_node (
  relay_id text primary key,
  region text not null,
  public_base_url text not null,
  internal_base_url text not null,
  status text not null default 'active'
    check (status in ('active', 'draining', 'disabled')),
  max_active_tunnels integer not null
    check (max_active_tunnels > 0),
  active_tunnel_count integer not null default 0
    check (active_tunnel_count >= 0),
  last_heartbeat_at timestamptz,
  heartbeat_expires_at timestamptz,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  check (relay_id = btrim(relay_id) and relay_id <> ''),
  check (region = btrim(region) and region <> ''),
  check (public_base_url = btrim(public_base_url) and public_base_url <> ''),
  check (internal_base_url = btrim(internal_base_url) and internal_base_url <> '')
);

create unique index if not exists mobile_tunnel_relay_node_public_url_idx
  on public.mobile_tunnel_relay_node (public_base_url);

create index if not exists mobile_tunnel_relay_node_assign_idx
  on public.mobile_tunnel_relay_node (
    region,
    status,
    heartbeat_expires_at desc,
    active_tunnel_count asc,
    relay_id asc
  );

create table if not exists public.mobile_tunnel (
  tunnel_id text primary key,
  user_id text not null,
  billing_subject_id text,
  relay_id text not null references public.mobile_tunnel_relay_node (relay_id),
  public_base_url text not null,
  status text not null default 'active'
    check (status in ('active', 'revoked')),
  assignment_version integer not null default 1
    check (assignment_version > 0),
  last_connected_at timestamptz,
  last_accessed_at timestamptz,
  disabled_at timestamptz,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  check (tunnel_id = btrim(tunnel_id) and tunnel_id <> ''),
  check (user_id = btrim(user_id) and user_id <> ''),
  check (billing_subject_id is null or (billing_subject_id = btrim(billing_subject_id) and billing_subject_id <> '')),
  check (public_base_url = btrim(public_base_url) and public_base_url <> ''),
  check (
    (status = 'active' and disabled_at is null)
    or
    (status = 'revoked' and disabled_at is not null)
  )
);

create unique index if not exists mobile_tunnel_active_user_idx
  on public.mobile_tunnel (user_id)
  where status = 'active' and disabled_at is null;

create index if not exists mobile_tunnel_relay_active_idx
  on public.mobile_tunnel (relay_id, status)
  where disabled_at is null;

create index if not exists mobile_tunnel_user_created_idx
  on public.mobile_tunnel (user_id, created_at desc);

create table if not exists public.mobile_tunnel_event (
  event_id bigserial primary key,
  tunnel_id text references public.mobile_tunnel (tunnel_id) on delete set null,
  relay_id text references public.mobile_tunnel_relay_node (relay_id) on delete set null,
  user_id text,
  event_type text not null,
  metadata jsonb not null default '{}'::jsonb,
  created_at timestamptz not null default now(),
  check (event_type = btrim(event_type) and event_type <> ''),
  check (user_id is null or (user_id = btrim(user_id) and user_id <> ''))
);

create index if not exists mobile_tunnel_event_tunnel_created_idx
  on public.mobile_tunnel_event (tunnel_id, created_at desc);

create index if not exists mobile_tunnel_event_relay_created_idx
  on public.mobile_tunnel_event (relay_id, created_at desc);

drop trigger if exists mobile_tunnel_relay_node_set_updated_at on public.mobile_tunnel_relay_node;
create trigger mobile_tunnel_relay_node_set_updated_at
before update on public.mobile_tunnel_relay_node
for each row execute procedure public.set_row_updated_at();

drop trigger if exists mobile_tunnel_set_updated_at on public.mobile_tunnel;
create trigger mobile_tunnel_set_updated_at
before update on public.mobile_tunnel
for each row execute procedure public.set_row_updated_at();

alter table public.mobile_tunnel_relay_node enable row level security;
alter table public.mobile_tunnel enable row level security;
alter table public.mobile_tunnel_event enable row level security;

revoke all on table public.mobile_tunnel_relay_node from anon, authenticated, public;
revoke all on table public.mobile_tunnel from anon, authenticated, public;
revoke all on table public.mobile_tunnel_event from anon, authenticated, public;
revoke all on sequence public.mobile_tunnel_event_event_id_seq from anon, authenticated, public;

grant usage on schema public to ctx_tunnel_service;
grant select, insert, update, delete on table public.mobile_tunnel_relay_node to ctx_tunnel_service;
grant select, insert, update, delete on table public.mobile_tunnel to ctx_tunnel_service;
grant select, insert, update, delete on table public.mobile_tunnel_event to ctx_tunnel_service;
grant usage, select on sequence public.mobile_tunnel_event_event_id_seq to ctx_tunnel_service;

grant all on table public.mobile_tunnel_relay_node to service_role;
grant all on table public.mobile_tunnel to service_role;
grant all on table public.mobile_tunnel_event to service_role;
grant usage, select on sequence public.mobile_tunnel_event_event_id_seq to service_role;

drop policy if exists mobile_tunnel_relay_node_service on public.mobile_tunnel_relay_node;
create policy mobile_tunnel_relay_node_service on public.mobile_tunnel_relay_node
  for all
  to service_role, ctx_tunnel_service
  using (true)
  with check (true);

drop policy if exists mobile_tunnel_service on public.mobile_tunnel;
create policy mobile_tunnel_service on public.mobile_tunnel
  for all
  to service_role, ctx_tunnel_service
  using (true)
  with check (true);

drop policy if exists mobile_tunnel_event_service on public.mobile_tunnel_event;
create policy mobile_tunnel_event_service on public.mobile_tunnel_event
  for all
  to service_role, ctx_tunnel_service
  using (true)
  with check (true);
