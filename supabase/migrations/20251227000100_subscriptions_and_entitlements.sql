-- Subscriptions + entitlements substrate (v0).
-- Source of truth:
-- - Supabase Auth for identity
-- - Stripe for billing status
--
-- Notes:
-- - Tables are readable by the owning user (authenticated) but writable only by service_role (via Edge Functions).

create table if not exists public.billing_profile (
  user_id uuid primary key references auth.users (id) on delete cascade,
  email text,
  stripe_customer_id text unique,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now()
);

create index if not exists billing_profile_stripe_customer_id_idx on public.billing_profile (stripe_customer_id);

create table if not exists public.billing_subscription (
  user_id uuid primary key references auth.users (id) on delete cascade,
  plan_type text not null default 'free_local', -- free_local | pro | team | enterprise
  status text not null default 'none',          -- none | active | trialing | past_due | canceled | unpaid | incomplete | ...
  stripe_subscription_id text unique,
  stripe_price_id text,
  current_period_end timestamptz,
  cancel_at_period_end boolean not null default false,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now()
);

create index if not exists billing_subscription_stripe_subscription_id_idx on public.billing_subscription (stripe_subscription_id);

-- Create minimal rows for new users so client reads never 404.
create or replace function public.handle_new_user_billing()
returns trigger
language plpgsql
security definer
set search_path = public
as $$
begin
  insert into public.billing_profile (user_id, email)
  values (new.id, new.email)
  on conflict (user_id) do update
    set email = excluded.email,
        updated_at = now();

  insert into public.billing_subscription (user_id)
  values (new.id)
  on conflict (user_id) do nothing;

  return new;
end;
$$;

drop trigger if exists on_auth_user_created_billing on auth.users;
create trigger on_auth_user_created_billing
after insert on auth.users
for each row execute procedure public.handle_new_user_billing();

-- Security: lock down writes; allow owner read.
alter table public.billing_profile enable row level security;
alter table public.billing_subscription enable row level security;

revoke all on table public.billing_profile from anon, authenticated, public;
revoke all on table public.billing_subscription from anon, authenticated, public;

grant all on table public.billing_profile to service_role;
grant all on table public.billing_subscription to service_role;

grant select on table public.billing_profile to authenticated;
grant select on table public.billing_subscription to authenticated;

drop policy if exists billing_profile_owner_read on public.billing_profile;
create policy billing_profile_owner_read on public.billing_profile
  for select
  to authenticated
  using (user_id = auth.uid());

drop policy if exists billing_subscription_owner_read on public.billing_subscription;
create policy billing_subscription_owner_read on public.billing_subscription
  for select
  to authenticated
  using (user_id = auth.uid());

drop policy if exists billing_profile_service_role on public.billing_profile;
create policy billing_profile_service_role on public.billing_profile
  for all
  to service_role
  using (true)
  with check (true);

drop policy if exists billing_subscription_service_role on public.billing_subscription;
create policy billing_subscription_service_role on public.billing_subscription
  for all
  to service_role
  using (true)
  with check (true);
