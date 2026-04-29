-- Team / Enterprise commercial context foundation.
-- Canonical commercial state is ctx-owned and function-mediated:
-- - ctx_account
-- - organization / membership / invite
-- - billing_subject / commerce_subscription / entitlement_grant
-- - external_identity_link
--
-- Existing billing_profile / billing_subscription remain as WIP placeholders
-- during migration and are mirrored into the new canonical tables.

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

create table if not exists public.ctx_account (
  id uuid primary key default gen_random_uuid(),
  primary_email text,
  display_name text,
  status text not null default 'active'
    check (status in ('active', 'disabled')),
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now()
);

create index if not exists ctx_account_primary_email_idx
  on public.ctx_account (lower(primary_email))
  where primary_email is not null;

create table if not exists public.organization (
  id uuid primary key default gen_random_uuid(),
  name text not null,
  slug text unique,
  status text not null default 'active'
    check (status in ('active', 'disabled')),
  created_by_account_id uuid references public.ctx_account (id) on delete set null,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now()
);

create table if not exists public.organization_membership (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references public.organization (id) on delete cascade,
  account_id uuid not null references public.ctx_account (id) on delete cascade,
  role text not null check (role in ('owner', 'admin', 'member')),
  status text not null default 'active'
    check (status in ('active', 'suspended')),
  invited_by_account_id uuid references public.ctx_account (id) on delete set null,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  unique (organization_id, account_id)
);

create index if not exists organization_membership_account_idx
  on public.organization_membership (account_id, status);

create table if not exists public.organization_invite (
  id uuid primary key default gen_random_uuid(),
  organization_id uuid not null references public.organization (id) on delete cascade,
  email text not null,
  role text not null check (role in ('owner', 'admin', 'member')),
  status text not null default 'pending'
    check (status in ('pending', 'accepted', 'revoked', 'expired')),
  invited_by_account_id uuid references public.ctx_account (id) on delete set null,
  accepted_by_account_id uuid references public.ctx_account (id) on delete set null,
  membership_id uuid references public.organization_membership (id) on delete set null,
  token_hash text not null unique,
  expires_at timestamptz not null,
  responded_at timestamptz,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now()
);

create unique index if not exists organization_invite_pending_email_idx
  on public.organization_invite (organization_id, lower(email))
  where status = 'pending';

create table if not exists public.billing_subject (
  id uuid primary key default gen_random_uuid(),
  subject_type text not null check (subject_type in ('account', 'org')),
  account_id uuid unique references public.ctx_account (id) on delete cascade,
  organization_id uuid unique references public.organization (id) on delete cascade,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  check (
    (
      subject_type = 'account'
      and account_id is not null
      and organization_id is null
    )
    or
    (
      subject_type = 'org'
      and organization_id is not null
      and account_id is null
    )
  )
);

create table if not exists public.commerce_subscription (
  id uuid primary key default gen_random_uuid(),
  billing_subject_id uuid not null references public.billing_subject (id) on delete cascade,
  provider text not null default 'stripe',
  plan_type text not null default 'free_local'
    check (plan_type in ('free_local', 'pro', 'team', 'enterprise')),
  status text not null default 'none',
  provider_customer_id text,
  provider_subscription_id text unique,
  provider_price_id text,
  seat_count integer not null default 1 check (seat_count >= 1),
  current_period_end timestamptz,
  cancel_at_period_end boolean not null default false,
  last_provider_event_id text,
  last_provider_event_created_at timestamptz,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  unique (billing_subject_id, provider)
);

create index if not exists commerce_subscription_status_idx
  on public.commerce_subscription (billing_subject_id, status, current_period_end);

alter table public.commerce_subscription
  add column if not exists seat_count integer not null default 1 check (seat_count >= 1);

create table if not exists public.entitlement_grant (
  id uuid primary key default gen_random_uuid(),
  billing_subject_id uuid not null references public.billing_subject (id) on delete cascade,
  feature_key text not null,
  feature_state text not null default 'enabled'
    check (feature_state in ('enabled', 'disabled')),
  source text not null default 'manual',
  starts_at timestamptz not null default now(),
  expires_at timestamptz,
  grace_expires_at timestamptz,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  unique (billing_subject_id, feature_key)
);

create index if not exists entitlement_grant_active_window_idx
  on public.entitlement_grant (billing_subject_id, starts_at, expires_at, grace_expires_at);

create table if not exists public.organization_admin_state (
  organization_id uuid primary key references public.organization (id) on delete cascade,
  seat_target integer check (seat_target is null or (seat_target >= 1 and seat_target <= 999)),
  policy_json jsonb not null default '{}'::jsonb,
  enterprise_setup_requested_at timestamptz,
  enterprise_setup_requested_by_account_id uuid references public.ctx_account (id) on delete set null,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now()
);

create table if not exists public.external_identity_link (
  id uuid primary key default gen_random_uuid(),
  account_id uuid not null references public.ctx_account (id) on delete cascade,
  provider text not null,
  external_subject text not null,
  external_email text,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  unique (provider, external_subject)
);

create index if not exists external_identity_link_account_idx
  on public.external_identity_link (account_id, provider);

drop trigger if exists ctx_account_set_updated_at on public.ctx_account;
create trigger ctx_account_set_updated_at
before update on public.ctx_account
for each row execute procedure public.set_row_updated_at();

drop trigger if exists organization_set_updated_at on public.organization;
create trigger organization_set_updated_at
before update on public.organization
for each row execute procedure public.set_row_updated_at();

drop trigger if exists organization_membership_set_updated_at on public.organization_membership;
create trigger organization_membership_set_updated_at
before update on public.organization_membership
for each row execute procedure public.set_row_updated_at();

drop trigger if exists organization_invite_set_updated_at on public.organization_invite;
create trigger organization_invite_set_updated_at
before update on public.organization_invite
for each row execute procedure public.set_row_updated_at();

drop trigger if exists billing_subject_set_updated_at on public.billing_subject;
create trigger billing_subject_set_updated_at
before update on public.billing_subject
for each row execute procedure public.set_row_updated_at();

drop trigger if exists commerce_subscription_set_updated_at on public.commerce_subscription;
create trigger commerce_subscription_set_updated_at
before update on public.commerce_subscription
for each row execute procedure public.set_row_updated_at();

drop trigger if exists entitlement_grant_set_updated_at on public.entitlement_grant;
create trigger entitlement_grant_set_updated_at
before update on public.entitlement_grant
for each row execute procedure public.set_row_updated_at();

drop trigger if exists organization_admin_state_set_updated_at on public.organization_admin_state;
create trigger organization_admin_state_set_updated_at
before update on public.organization_admin_state
for each row execute procedure public.set_row_updated_at();

drop trigger if exists external_identity_link_set_updated_at on public.external_identity_link;
create trigger external_identity_link_set_updated_at
before update on public.external_identity_link
for each row execute procedure public.set_row_updated_at();

create or replace function public.handle_auth_user_commercial_sync()
returns trigger
language plpgsql
security definer
set search_path = public
as $$
declare
  resolved_account_id uuid;
begin
  select external_identity_link.account_id
    into resolved_account_id
  from public.external_identity_link
  where provider = 'supabase_auth'
    and external_subject = new.id::text
  limit 1;

  if resolved_account_id is null then
    insert into public.ctx_account (primary_email)
    values (new.email)
    returning id into resolved_account_id;

    insert into public.external_identity_link (
      account_id,
      provider,
      external_subject,
      external_email
    )
    values (
      resolved_account_id,
      'supabase_auth',
      new.id::text,
      new.email
    )
    on conflict (provider, external_subject) do update
      set account_id = excluded.account_id,
          external_email = excluded.external_email,
          updated_at = now();
  else
    update public.ctx_account
      set primary_email = coalesce(new.email, ctx_account.primary_email),
          updated_at = now()
      where id = resolved_account_id;

    update public.external_identity_link
      set external_email = new.email,
          updated_at = now()
      where provider = 'supabase_auth'
        and external_subject = new.id::text;
  end if;

  insert into public.billing_subject (subject_type, account_id)
  values ('account', resolved_account_id)
  on conflict (account_id) do nothing;

  return new;
end;
$$;

drop trigger if exists on_auth_user_commercial_sync on auth.users;
create trigger on_auth_user_commercial_sync
after insert or update of email on auth.users
for each row execute procedure public.handle_auth_user_commercial_sync();

create or replace function public.handle_new_organization_billing_subject()
returns trigger
language plpgsql
security definer
set search_path = public
as $$
begin
  insert into public.billing_subject (subject_type, organization_id)
  values ('org', new.id)
  on conflict (organization_id) do nothing;
  return new;
end;
$$;

drop trigger if exists on_organization_created_billing_subject on public.organization;
create trigger on_organization_created_billing_subject
after insert on public.organization
for each row execute procedure public.handle_new_organization_billing_subject();

with auth_users_to_backfill as (
  select
    auth.users.id as auth_user_id,
    auth.users.email as auth_user_email,
    gen_random_uuid() as account_id
  from auth.users
  left join public.external_identity_link
    on public.external_identity_link.provider = 'supabase_auth'
   and public.external_identity_link.external_subject = auth.users.id::text
  where public.external_identity_link.id is null
),
inserted_accounts as (
  insert into public.ctx_account (id, primary_email)
  select account_id, auth_user_email
  from auth_users_to_backfill
  on conflict (id) do nothing
  returning id
)
insert into public.external_identity_link (
  account_id,
  provider,
  external_subject,
  external_email
)
select
  account_id,
  'supabase_auth',
  auth_user_id::text,
  auth_user_email
from auth_users_to_backfill
on conflict (provider, external_subject) do update
  set account_id = excluded.account_id,
      external_email = excluded.external_email,
      updated_at = now();

update public.ctx_account
set primary_email = coalesce(auth.users.email, public.ctx_account.primary_email),
    updated_at = now()
from auth.users
join public.external_identity_link
  on public.external_identity_link.provider = 'supabase_auth'
 and public.external_identity_link.external_subject = auth.users.id::text
where public.ctx_account.id = public.external_identity_link.account_id;

insert into public.billing_subject (subject_type, account_id)
select 'account', public.external_identity_link.account_id
from public.external_identity_link
where public.external_identity_link.provider = 'supabase_auth'
on conflict (account_id) do nothing;

insert into public.billing_subject (subject_type, organization_id)
select 'org', public.organization.id
from public.organization
on conflict (organization_id) do nothing;

insert into public.commerce_subscription (
  billing_subject_id,
  provider,
  plan_type,
  status,
  provider_customer_id,
  provider_subscription_id,
  provider_price_id,
  seat_count,
  current_period_end,
  cancel_at_period_end,
  last_provider_event_id,
  last_provider_event_created_at,
  created_at,
  updated_at
)
select
  public.billing_subject.id,
  'stripe',
  coalesce(nullif(public.billing_subscription.plan_type, ''), 'free_local'),
  coalesce(nullif(public.billing_subscription.status, ''), 'none'),
  public.billing_profile.stripe_customer_id,
  public.billing_subscription.stripe_subscription_id,
  public.billing_subscription.stripe_price_id,
  1,
  public.billing_subscription.current_period_end,
  coalesce(public.billing_subscription.cancel_at_period_end, false),
  public.billing_subscription.stripe_last_event_id,
  public.billing_subscription.stripe_last_event_created,
  coalesce(public.billing_subscription.created_at, now()),
  coalesce(public.billing_subscription.updated_at, now())
from public.billing_subscription
join public.external_identity_link
  on public.external_identity_link.provider = 'supabase_auth'
 and public.external_identity_link.external_subject = public.billing_subscription.user_id::text
join public.billing_subject
  on public.billing_subject.subject_type = 'account'
 and public.billing_subject.account_id = public.external_identity_link.account_id
left join public.billing_profile
  on public.billing_profile.user_id = public.billing_subscription.user_id
on conflict (billing_subject_id, provider) do update
  set plan_type = excluded.plan_type,
      status = excluded.status,
      provider_customer_id = excluded.provider_customer_id,
      provider_subscription_id = excluded.provider_subscription_id,
      provider_price_id = excluded.provider_price_id,
      seat_count = excluded.seat_count,
      current_period_end = excluded.current_period_end,
      cancel_at_period_end = excluded.cancel_at_period_end,
      last_provider_event_id = excluded.last_provider_event_id,
      last_provider_event_created_at = excluded.last_provider_event_created_at,
      updated_at = excluded.updated_at;

alter table public.ctx_account enable row level security;
alter table public.organization enable row level security;
alter table public.organization_membership enable row level security;
alter table public.organization_invite enable row level security;
alter table public.billing_subject enable row level security;
alter table public.commerce_subscription enable row level security;
alter table public.entitlement_grant enable row level security;
alter table public.organization_admin_state enable row level security;
alter table public.external_identity_link enable row level security;

revoke all on table public.ctx_account from anon, authenticated, public;
revoke all on table public.organization from anon, authenticated, public;
revoke all on table public.organization_membership from anon, authenticated, public;
revoke all on table public.organization_invite from anon, authenticated, public;
revoke all on table public.billing_subject from anon, authenticated, public;
revoke all on table public.commerce_subscription from anon, authenticated, public;
revoke all on table public.entitlement_grant from anon, authenticated, public;
revoke all on table public.organization_admin_state from anon, authenticated, public;
revoke all on table public.external_identity_link from anon, authenticated, public;

grant all on table public.ctx_account to service_role;
grant all on table public.organization to service_role;
grant all on table public.organization_membership to service_role;
grant all on table public.organization_invite to service_role;
grant all on table public.billing_subject to service_role;
grant all on table public.commerce_subscription to service_role;
grant all on table public.entitlement_grant to service_role;
grant all on table public.organization_admin_state to service_role;
grant all on table public.external_identity_link to service_role;

grant select on table public.ctx_account to authenticated;
grant select on table public.organization to authenticated;
grant select on table public.organization_membership to authenticated;
grant select (
  id,
  organization_id,
  email,
  role,
  status,
  invited_by_account_id,
  accepted_by_account_id,
  membership_id,
  expires_at,
  responded_at,
  created_at,
  updated_at
) on table public.organization_invite to authenticated;
grant select on table public.billing_subject to authenticated;
grant select on table public.commerce_subscription to authenticated;
grant select on table public.entitlement_grant to authenticated;
grant select on table public.organization_admin_state to authenticated;
grant select on table public.external_identity_link to authenticated;

drop policy if exists ctx_account_authenticated_read on public.ctx_account;
create policy ctx_account_authenticated_read on public.ctx_account
  for select
  to authenticated
  using (
    exists (
      select 1
      from public.external_identity_link
      where public.external_identity_link.account_id = public.ctx_account.id
        and public.external_identity_link.provider = 'supabase_auth'
        and public.external_identity_link.external_subject = auth.uid()::text
    )
  );

drop policy if exists organization_authenticated_read on public.organization;
create policy organization_authenticated_read on public.organization
  for select
  to authenticated
  using (
    exists (
      select 1
      from public.organization_membership
      join public.external_identity_link
        on public.external_identity_link.account_id = public.organization_membership.account_id
      where public.organization_membership.organization_id = public.organization.id
        and public.organization_membership.status = 'active'
        and public.external_identity_link.provider = 'supabase_auth'
        and public.external_identity_link.external_subject = auth.uid()::text
    )
  );

drop policy if exists organization_membership_authenticated_read on public.organization_membership;
create policy organization_membership_authenticated_read on public.organization_membership
  for select
  to authenticated
  using (
    exists (
      select 1
      from public.external_identity_link
      where public.external_identity_link.account_id = public.organization_membership.account_id
        and public.external_identity_link.provider = 'supabase_auth'
        and public.external_identity_link.external_subject = auth.uid()::text
    )
  );

drop policy if exists organization_invite_authenticated_read on public.organization_invite;
create policy organization_invite_authenticated_read on public.organization_invite
  for select
  to authenticated
  using (
    exists (
      select 1
      from public.organization_membership
      join public.external_identity_link
        on public.external_identity_link.account_id = public.organization_membership.account_id
      where public.organization_membership.organization_id = public.organization_invite.organization_id
        and public.organization_membership.status = 'active'
        and public.organization_membership.role in ('owner', 'admin')
        and public.external_identity_link.provider = 'supabase_auth'
        and public.external_identity_link.external_subject = auth.uid()::text
    )
  );

drop policy if exists billing_subject_authenticated_read on public.billing_subject;
create policy billing_subject_authenticated_read on public.billing_subject
  for select
  to authenticated
  using (
    (
      public.billing_subject.subject_type = 'account'
      and exists (
        select 1
        from public.external_identity_link
        where public.external_identity_link.account_id = public.billing_subject.account_id
          and public.external_identity_link.provider = 'supabase_auth'
          and public.external_identity_link.external_subject = auth.uid()::text
      )
    )
    or
    (
      public.billing_subject.subject_type = 'org'
      and exists (
        select 1
        from public.organization_membership
        join public.external_identity_link
          on public.external_identity_link.account_id = public.organization_membership.account_id
        where public.organization_membership.organization_id = public.billing_subject.organization_id
          and public.organization_membership.status = 'active'
          and public.external_identity_link.provider = 'supabase_auth'
          and public.external_identity_link.external_subject = auth.uid()::text
      )
    )
  );

drop policy if exists commerce_subscription_authenticated_read on public.commerce_subscription;
create policy commerce_subscription_authenticated_read on public.commerce_subscription
  for select
  to authenticated
  using (
    exists (
      select 1
      from public.billing_subject
      where public.billing_subject.id = public.commerce_subscription.billing_subject_id
        and (
          (
            public.billing_subject.subject_type = 'account'
            and exists (
              select 1
              from public.external_identity_link
              where public.external_identity_link.account_id = public.billing_subject.account_id
                and public.external_identity_link.provider = 'supabase_auth'
                and public.external_identity_link.external_subject = auth.uid()::text
            )
          )
          or
          (
            public.billing_subject.subject_type = 'org'
            and exists (
              select 1
              from public.organization_membership
              join public.external_identity_link
                on public.external_identity_link.account_id = public.organization_membership.account_id
              where public.organization_membership.organization_id = public.billing_subject.organization_id
                and public.organization_membership.status = 'active'
                and public.organization_membership.role in ('owner', 'admin')
                and public.external_identity_link.provider = 'supabase_auth'
                and public.external_identity_link.external_subject = auth.uid()::text
            )
          )
        )
    )
  );

drop policy if exists entitlement_grant_authenticated_read on public.entitlement_grant;
create policy entitlement_grant_authenticated_read on public.entitlement_grant
  for select
  to authenticated
  using (
    exists (
      select 1
      from public.billing_subject
      where public.billing_subject.id = public.entitlement_grant.billing_subject_id
        and (
          (
            public.billing_subject.subject_type = 'account'
            and exists (
              select 1
              from public.external_identity_link
              where public.external_identity_link.account_id = public.billing_subject.account_id
                and public.external_identity_link.provider = 'supabase_auth'
                and public.external_identity_link.external_subject = auth.uid()::text
            )
          )
          or
          (
            public.billing_subject.subject_type = 'org'
            and exists (
              select 1
              from public.organization_membership
              join public.external_identity_link
                on public.external_identity_link.account_id = public.organization_membership.account_id
              where public.organization_membership.organization_id = public.billing_subject.organization_id
                and public.organization_membership.status = 'active'
                and public.external_identity_link.provider = 'supabase_auth'
                and public.external_identity_link.external_subject = auth.uid()::text
            )
          )
        )
    )
  );

drop policy if exists external_identity_link_authenticated_read on public.external_identity_link;
create policy external_identity_link_authenticated_read on public.external_identity_link
  for select
  to authenticated
  using (
    public.external_identity_link.provider = 'supabase_auth'
    and public.external_identity_link.external_subject = auth.uid()::text
  );

drop policy if exists organization_admin_state_authenticated_read on public.organization_admin_state;
create policy organization_admin_state_authenticated_read on public.organization_admin_state
  for select
  to authenticated
  using (
    exists (
      select 1
      from public.organization_membership
      join public.external_identity_link
        on public.external_identity_link.account_id = public.organization_membership.account_id
      where public.organization_membership.organization_id = public.organization_admin_state.organization_id
        and public.organization_membership.status = 'active'
        and public.organization_membership.role in ('owner', 'admin')
        and public.external_identity_link.provider = 'supabase_auth'
        and public.external_identity_link.external_subject = auth.uid()::text
    )
  );

drop policy if exists ctx_account_service_role on public.ctx_account;
create policy ctx_account_service_role on public.ctx_account
  for all
  to service_role
  using (true)
  with check (true);

drop policy if exists organization_service_role on public.organization;
create policy organization_service_role on public.organization
  for all
  to service_role
  using (true)
  with check (true);

drop policy if exists organization_membership_service_role on public.organization_membership;
create policy organization_membership_service_role on public.organization_membership
  for all
  to service_role
  using (true)
  with check (true);

drop policy if exists organization_invite_service_role on public.organization_invite;
create policy organization_invite_service_role on public.organization_invite
  for all
  to service_role
  using (true)
  with check (true);

drop policy if exists billing_subject_service_role on public.billing_subject;
create policy billing_subject_service_role on public.billing_subject
  for all
  to service_role
  using (true)
  with check (true);

drop policy if exists commerce_subscription_service_role on public.commerce_subscription;
create policy commerce_subscription_service_role on public.commerce_subscription
  for all
  to service_role
  using (true)
  with check (true);

drop policy if exists entitlement_grant_service_role on public.entitlement_grant;
create policy entitlement_grant_service_role on public.entitlement_grant
  for all
  to service_role
  using (true)
  with check (true);

drop policy if exists organization_admin_state_service_role on public.organization_admin_state;
create policy organization_admin_state_service_role on public.organization_admin_state
  for all
  to service_role
  using (true)
  with check (true);

drop policy if exists external_identity_link_service_role on public.external_identity_link;
create policy external_identity_link_service_role on public.external_identity_link
  for all
  to service_role
  using (true)
  with check (true);
