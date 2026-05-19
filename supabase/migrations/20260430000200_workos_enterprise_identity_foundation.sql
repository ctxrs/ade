-- WorkOS Enterprise identity foundation.
-- WorkOS is not the ctx session authority. These tables persist Enterprise
-- identity setup state so server-side functions can safely create Admin Portal
-- links and ingest WorkOS events.

create table if not exists public.workos_organization_link (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references public.organization (id) on delete cascade,
  workos_organization_id text,
  external_id text not null,
  status text not null default 'pending'
    check (status in ('pending', 'active', 'disabled')),
  ensure_token text not null,
  created_by_account_id uuid references public.ctx_account (id) on delete set null,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now()
);

create unique index if not exists workos_organization_link_organization_id_uidx
  on public.workos_organization_link (organization_id);

create unique index if not exists workos_organization_link_workos_org_id_uidx
  on public.workos_organization_link (workos_organization_id)
  where workos_organization_id is not null;

create unique index if not exists workos_organization_link_external_id_uidx
  on public.workos_organization_link (external_id);

create unique index if not exists workos_organization_link_ensure_token_uidx
  on public.workos_organization_link (ensure_token);

create index if not exists workos_organization_link_status_idx
  on public.workos_organization_link (status, updated_at desc);

create table if not exists public.workos_identity_event (
  id uuid primary key default gen_random_uuid(),
  workos_event_id text not null,
  event_key text not null,
  workos_organization_id text,
  organization_id uuid references public.organization (id) on delete set null,
  workos_created_at timestamptz,
  payload jsonb not null default '{}'::jsonb,
  received_at timestamptz not null default now()
);

create unique index if not exists workos_identity_event_event_id_uidx
  on public.workos_identity_event (workos_event_id);

create index if not exists workos_identity_event_key_received_idx
  on public.workos_identity_event (event_key, received_at desc);

create index if not exists workos_identity_event_workos_org_idx
  on public.workos_identity_event (workos_organization_id, received_at desc)
  where workos_organization_id is not null;

create table if not exists public.workos_identity_resource_status (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references public.organization (id) on delete cascade,
  workos_organization_id text not null,
  resource_kind text not null
    check (resource_kind in ('domain_verification', 'sso_connection', 'directory_sync')),
  external_resource_id text,
  resource_key text not null default '__ctx_unassigned__',
  status text not null default 'unknown'
    check (status in ('unknown', 'pending', 'configured', 'active', 'inactive', 'failed')),
  status_source text not null default 'webhook'
    check (status_source in ('admin_portal', 'webhook', 'operator')),
  last_workos_event_id text,
  source_workos_created_at timestamptz,
  last_seen_at timestamptz not null default now(),
  metadata jsonb not null default '{}'::jsonb,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now()
);

create unique index if not exists workos_identity_resource_status_resource_uidx
  on public.workos_identity_resource_status (
    organization_id,
    resource_kind,
    resource_key
  );

create index if not exists workos_identity_resource_status_org_kind_idx
  on public.workos_identity_resource_status (
    organization_id,
    resource_kind,
    status
  );

alter table public.workos_organization_link enable row level security;

alter table public.workos_identity_event enable row level security;

alter table public.workos_identity_resource_status enable row level security;

revoke all on table public.workos_organization_link from anon, authenticated, public;

revoke all on table public.workos_identity_event from anon, authenticated, public;

revoke all on table public.workos_identity_resource_status from anon, authenticated, public;

grant all on table public.workos_organization_link to service_role;

grant all on table public.workos_identity_event to service_role;

grant all on table public.workos_identity_resource_status to service_role;

drop policy if exists workos_organization_link_service_role on public.workos_organization_link;

create policy workos_organization_link_service_role on public.workos_organization_link
  for all
  to service_role
  using (true)
  with check (true);

drop policy if exists workos_identity_event_service_role on public.workos_identity_event;

create policy workos_identity_event_service_role on public.workos_identity_event
  for all
  to service_role
  using (true)
  with check (true);

drop policy if exists workos_identity_resource_status_service_role
  on public.workos_identity_resource_status;

create policy workos_identity_resource_status_service_role
  on public.workos_identity_resource_status
  for all
  to service_role
  using (true)
  with check (true);

create or replace function public.reserve_workos_organization_link(
  p_organization_id uuid,
  p_created_by_account_id uuid,
  p_external_id text,
  p_ensure_token text
)
returns table (
  id uuid,
  organization_id uuid,
  workos_organization_id text,
  external_id text,
  status text,
  ensure_token text,
  created_by_account_id uuid,
  created_at timestamptz,
  updated_at timestamptz,
  reserved_now boolean
)
language plpgsql
security definer
set search_path = public
as $$
declare
  existing_id uuid;
begin
  perform pg_advisory_xact_lock(0, hashtext(p_organization_id::text));

  select wol.id
    into existing_id
  from public.workos_organization_link wol
  where wol.organization_id = p_organization_id;

  if existing_id is null then
    insert into public.workos_organization_link (
      organization_id,
      external_id,
      ensure_token,
      created_by_account_id
    )
    values (
      p_organization_id,
      p_external_id,
      p_ensure_token,
      p_created_by_account_id
    )
    returning workos_organization_link.id into existing_id;

    return query
      select
        wol.id,
        wol.organization_id,
        wol.workos_organization_id,
        wol.external_id,
        wol.status,
        wol.ensure_token,
        wol.created_by_account_id,
        wol.created_at,
        wol.updated_at,
        true
      from public.workos_organization_link wol
      where wol.id = existing_id;
  elsif exists (
    select 1
    from public.workos_organization_link wol
    where wol.id = existing_id
      and wol.status = 'pending'
      and wol.workos_organization_id is null
      and wol.updated_at < now() - interval '10 minutes'
  ) then
    update public.workos_organization_link wol
      set ensure_token = p_ensure_token,
          created_by_account_id = p_created_by_account_id,
          updated_at = now()
      where wol.id = existing_id;

    return query
      select
        wol.id,
        wol.organization_id,
        wol.workos_organization_id,
        wol.external_id,
        wol.status,
        wol.ensure_token,
        wol.created_by_account_id,
        wol.created_at,
        wol.updated_at,
        true
      from public.workos_organization_link wol
      where wol.id = existing_id;
  else
    return query
      select
        wol.id,
        wol.organization_id,
        wol.workos_organization_id,
        wol.external_id,
        wol.status,
        wol.ensure_token,
        wol.created_by_account_id,
        wol.created_at,
        wol.updated_at,
        false
      from public.workos_organization_link wol
      where wol.id = existing_id;
  end if;
end;
$$;

revoke all on function public.reserve_workos_organization_link(
  uuid,
  uuid,
  text,
  text
) from public, anon, authenticated;

grant execute on function public.reserve_workos_organization_link(
  uuid,
  uuid,
  text,
  text
) to service_role;

create or replace function public.record_workos_identity_event(
  p_workos_event_id text,
  p_event_key text,
  p_workos_organization_id text,
  p_organization_id uuid,
  p_workos_created_at timestamptz,
  p_payload jsonb,
  p_resource_kind text default null,
  p_external_resource_id text default null,
  p_resource_status text default null,
  p_resource_metadata jsonb default '{}'::jsonb
)
returns table (
  event_write text,
  resource_applied boolean
)
language plpgsql
security definer
set search_path = public
as $$
declare
  inserted_event_id uuid;
  affected_resource_rows integer := 0;
  resource_key text;
begin
  insert into public.workos_identity_event (
    workos_event_id,
    event_key,
    workos_organization_id,
    organization_id,
    workos_created_at,
    payload
  )
  values (
    p_workos_event_id,
    p_event_key,
    p_workos_organization_id,
    p_organization_id,
    p_workos_created_at,
    coalesce(p_payload, '{}'::jsonb)
  )
  on conflict (workos_event_id) do nothing
  returning id into inserted_event_id;

  if inserted_event_id is null then
    return query select 'duplicate'::text, false;
    return;
  end if;

  if p_organization_id is not null
     and p_workos_organization_id is not null
     and p_resource_kind is not null
     and p_resource_status is not null
  then
    resource_key := coalesce(p_external_resource_id, '__ctx_unassigned__');

    insert into public.workos_identity_resource_status (
      organization_id,
      workos_organization_id,
      resource_kind,
      external_resource_id,
      resource_key,
      status,
      status_source,
      last_workos_event_id,
      source_workos_created_at,
      last_seen_at,
      metadata,
      updated_at
    )
    values (
      p_organization_id,
      p_workos_organization_id,
      p_resource_kind,
      p_external_resource_id,
      resource_key,
      p_resource_status,
      'webhook',
      p_workos_event_id,
      p_workos_created_at,
      now(),
      coalesce(p_resource_metadata, '{}'::jsonb),
      now()
    )
    on conflict (organization_id, resource_kind, resource_key) do update
    set workos_organization_id = excluded.workos_organization_id,
        external_resource_id = excluded.external_resource_id,
        status = excluded.status,
        status_source = excluded.status_source,
        last_workos_event_id = excluded.last_workos_event_id,
        source_workos_created_at = excluded.source_workos_created_at,
        last_seen_at = excluded.last_seen_at,
        metadata = excluded.metadata,
        updated_at = excluded.updated_at
    where public.workos_identity_resource_status.source_workos_created_at is null
       or (
         excluded.source_workos_created_at is not null
         and excluded.source_workos_created_at >= public.workos_identity_resource_status.source_workos_created_at
       );

    get diagnostics affected_resource_rows = row_count;
  end if;

  return query select 'inserted'::text, affected_resource_rows > 0;
end;
$$;

revoke all on function public.record_workos_identity_event(
  text,
  text,
  text,
  uuid,
  timestamptz,
  jsonb,
  text,
  text,
  text,
  jsonb
) from public, anon, authenticated;

grant execute on function public.record_workos_identity_event(
  text,
  text,
  text,
  uuid,
  timestamptz,
  jsonb,
  text,
  text,
  text,
  jsonb
) to service_role;
