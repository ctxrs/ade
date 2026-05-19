-- Team / Enterprise launch readiness schema.
-- Adds invite delivery state and a service-role commercial operations ledger.

alter table public.organization_invite
  add column if not exists delivery_status text not null default 'pending'
    check (delivery_status in ('pending', 'sent', 'failed')),
  add column if not exists delivery_attempted_at timestamptz,
  add column if not exists delivery_error_code text;

create table if not exists public.commercial_event (
  id uuid primary key default gen_random_uuid(),
  event_key text not null,
  subject_type text not null check (subject_type in ('account', 'org', 'install', 'unknown')),
  subject_id text,
  account_id uuid references public.ctx_account (id) on delete set null,
  organization_id uuid references public.organization (id) on delete set null,
  provider text,
  provider_customer_id text,
  provider_subscription_id text,
  provider_session_id text,
  status text not null default 'ok',
  error_code text,
  metadata jsonb not null default '{}'::jsonb,
  created_at timestamptz not null default now()
);

create index if not exists commercial_event_subject_created_idx
  on public.commercial_event (subject_type, subject_id, created_at desc);

create index if not exists commercial_event_key_created_idx
  on public.commercial_event (event_key, created_at desc);

alter table public.commercial_event enable row level security;

revoke all on table public.commercial_event from anon, authenticated, public;

grant all on table public.commercial_event to service_role;

drop policy if exists commercial_event_service_role on public.commercial_event;

create policy commercial_event_service_role on public.commercial_event
  for all
  to service_role
  using (true)
  with check (true);

grant select (
  id,
  organization_id,
  email,
  role,
  status,
  delivery_status,
  delivery_attempted_at,
  delivery_error_code,
  invited_by_account_id,
  accepted_by_account_id,
  membership_id,
  expires_at,
  responded_at,
  created_at,
  updated_at
) on table public.organization_invite to authenticated;
