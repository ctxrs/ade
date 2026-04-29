-- LLM token relay control-plane and ledger substrate (v1).
--
-- Source of truth:
-- - ctx-owned ids are canonical for product identity and billing subjects.
-- - WorkOS, Stripe, and Supabase Auth ids are external facts only.
-- - Provider secrets are never stored here; route rows may store credential refs.
-- - Money-bearing request/accounting tables are append-only. Corrections use
--   compensating events tied to original request/reservation ids.

do $$
begin
  create type public.ctx_access_context_kind as enum ('personal', 'org');
exception when duplicate_object then null;
end $$;

do $$
begin
  create type public.ctx_plan_kind as enum ('free', 'pro', 'team', 'enterprise');
exception when duplicate_object then null;
end $$;

do $$
begin
  create type public.ctx_membership_role as enum ('owner', 'admin', 'member', 'billing_admin', 'auditor');
exception when duplicate_object then null;
end $$;

do $$
begin
  create type public.ctx_membership_status as enum ('invited', 'active', 'suspended', 'removed');
exception when duplicate_object then null;
end $$;

do $$
begin
  create type public.ctx_billing_subject_kind as enum ('personal_account', 'org');
exception when duplicate_object then null;
end $$;

do $$
begin
  create type public.ctx_route_type as enum ('ctx_managed', 'user_managed', 'customer_gateway');
exception when duplicate_object then null;
end $$;

do $$
begin
  create type public.ctx_credential_owner as enum ('ctx', 'user', 'customer');
exception when duplicate_object then null;
end $$;

do $$
begin
  create type public.ctx_route_auth_method as enum ('ctx_provider_key', 'oauth', 'api_key', 'gateway_token', 'mtls');
exception when duplicate_object then null;
end $$;

do $$
begin
  create type public.ctx_governance_level as enum ('hard_enforced', 'policy_enforced_local', 'receipt_enforced');
exception when duplicate_object then null;
end $$;

do $$
begin
  create type public.ctx_route_status as enum ('enabled', 'disabled', 'planned');
exception when duplicate_object then null;
end $$;

do $$
begin
  create type public.ctx_credit_source as enum ('promo_trial', 'subscription_included', 'enterprise_commit', 'prepaid_top_up');
exception when duplicate_object then null;
end $$;

do $$
begin
  create type public.ctx_credit_event_kind as enum (
    'grant',
    'reserve',
    'finalize',
    'release',
    'expire',
    'void',
    'reconciliation_adjustment'
  );
exception when duplicate_object then null;
end $$;

do $$
begin
  create type public.ctx_relay_request_state as enum (
    'rejected_preflight',
    'reserved',
    'provider_started',
    'stream_completed',
    'stream_broken_usage_unknown',
    'finalized',
    'voided',
    'reconciled'
  );
exception when duplicate_object then null;
end $$;

do $$
begin
  create type public.ctx_usage_event_kind as enum (
    'rejected_preflight',
    'reserved',
    'provider_started',
    'stream_completed',
    'stream_broken_usage_unknown',
    'finalized',
    'voided',
    'reconciled'
  );
exception when duplicate_object then null;
end $$;

create table if not exists public.ctx_users (
  id text primary key,
  primary_email text,
  workos_user_id text unique,
  supabase_user_id uuid unique,
  status text not null default 'active',
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now()
);

create table if not exists public.ctx_accounts (
  id text primary key,
  ctx_user_id text not null references public.ctx_users (id),
  stripe_customer_id text unique,
  status text not null default 'active',
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now()
);

create table if not exists public.ctx_orgs (
  id text primary key,
  name text not null,
  slug text unique,
  plan_kind public.ctx_plan_kind not null default 'team',
  workos_org_id text unique,
  stripe_customer_id text unique,
  status text not null default 'active',
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now()
);

create table if not exists public.ctx_memberships (
  id text primary key,
  ctx_org_id text not null references public.ctx_orgs (id),
  ctx_user_id text not null references public.ctx_users (id),
  role public.ctx_membership_role not null default 'member',
  status public.ctx_membership_status not null default 'active',
  workos_membership_id text unique,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  unique (ctx_org_id, ctx_user_id)
);

create table if not exists public.billing_subjects (
  id text primary key,
  kind public.ctx_billing_subject_kind not null,
  ctx_account_id text unique references public.ctx_accounts (id),
  ctx_org_id text unique references public.ctx_orgs (id),
  currency text not null default 'usd',
  status text not null default 'active',
  stripe_customer_id text unique,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  check (
    (kind = 'personal_account' and ctx_account_id is not null and ctx_org_id is null)
    or
    (kind = 'org' and ctx_org_id is not null and ctx_account_id is null)
  )
);

create table if not exists public.billing_entitlements (
  id text primary key,
  billing_subject_id text not null references public.billing_subjects (id),
  plan_kind public.ctx_plan_kind not null,
  status text not null,
  stripe_subscription_id text unique,
  stripe_price_id text,
  current_period_start timestamptz,
  current_period_end timestamptz,
  metadata jsonb not null default '{}'::jsonb,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now()
);

create table if not exists public.billing_spend_limits (
  id text primary key,
  billing_subject_id text not null references public.billing_subjects (id),
  ctx_user_id text references public.ctx_users (id),
  status text not null default 'active',
  period_start timestamptz not null,
  period_end timestamptz not null,
  hard_limit_cents bigint not null check (hard_limit_cents > 0),
  metadata jsonb not null default '{}'::jsonb,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  check (status in ('active', 'disabled')),
  check (period_end > period_start)
);

create table if not exists public.route_configs (
  id text primary key,
  billing_subject_id text not null references public.billing_subjects (id),
  route_type public.ctx_route_type not null,
  credential_owner public.ctx_credential_owner not null,
  auth_method public.ctx_route_auth_method not null,
  governance_level public.ctx_governance_level not null,
  status public.ctx_route_status not null default 'enabled',
  provider_id text not null,
  name text,
  credential_ref text,
  model_allowlist jsonb not null default '[]'::jsonb,
  route_config jsonb not null default '{}'::jsonb,
  created_at timestamptz not null default now(),
  updated_at timestamptz not null default now(),
  check (credential_ref is null or length(trim(credential_ref)) > 0),
  check (
    (route_type = 'ctx_managed' and credential_owner = 'ctx' and auth_method = 'ctx_provider_key' and governance_level = 'hard_enforced')
    or
    (route_type = 'user_managed' and credential_owner = 'user' and auth_method in ('oauth', 'api_key') and governance_level = 'policy_enforced_local')
    or
    (route_type = 'customer_gateway' and credential_owner = 'customer' and auth_method in ('gateway_token', 'mtls') and governance_level = 'receipt_enforced')
  )
);

comment on column public.route_configs.credential_ref is
  'Reference to a secret in external secret storage. Provider keys must not be stored in Postgres.';

create table if not exists public.route_policy_versions (
  id text primary key,
  billing_subject_id text not null references public.billing_subjects (id),
  policy_version text not null,
  is_active boolean not null default false,
  policy jsonb not null,
  signed_snapshot_public_key_ref text,
  created_at timestamptz not null default now(),
  unique (billing_subject_id, policy_version)
);

create table if not exists public.pricing_catalog_versions (
  version text primary key,
  status text not null default 'draft',
  currency text not null default 'usd',
  effective_at timestamptz,
  created_at timestamptz not null default now()
);

create table if not exists public.model_prices (
  id text primary key,
  pricing_version text not null references public.pricing_catalog_versions (version),
  provider_id text not null,
  model_id text not null,
  input_cost_micros_per_1k_tokens bigint not null check (input_cost_micros_per_1k_tokens >= 0),
  output_cost_micros_per_1k_tokens bigint not null check (output_cost_micros_per_1k_tokens >= 0),
  customer_input_micros_per_1k_tokens bigint not null check (customer_input_micros_per_1k_tokens >= 0),
  customer_output_micros_per_1k_tokens bigint not null check (customer_output_micros_per_1k_tokens >= 0),
  request_overhead_input_tokens integer not null default 0 check (request_overhead_input_tokens >= 0),
  function_schema_overhead_input_tokens integer not null default 0 check (function_schema_overhead_input_tokens >= 0),
  created_at timestamptz not null default now(),
  unique (pricing_version, provider_id, model_id)
);

create table if not exists public.credit_grants (
  id text primary key,
  billing_subject_id text not null references public.billing_subjects (id),
  source public.ctx_credit_source not null,
  total_cents bigint not null check (total_cents > 0),
  issued_at timestamptz not null default now(),
  expires_at timestamptz,
  external_ref text,
  metadata jsonb not null default '{}'::jsonb,
  created_at timestamptz not null default now(),
  check (expires_at is null or expires_at > issued_at)
);

create table if not exists public.credit_ledger_events (
  id text primary key,
  billing_subject_id text not null references public.billing_subjects (id),
  credit_grant_id text references public.credit_grants (id),
  request_id text,
  reservation_id text,
  event_kind public.ctx_credit_event_kind not null,
  amount_cents bigint not null check (amount_cents <> 0),
  idempotency_key text unique,
  metadata jsonb not null default '{}'::jsonb,
  created_at timestamptz not null default now()
);

create table if not exists public.usage_reservations (
  id text primary key,
  request_id text not null unique,
  run_grant_jti text not null unique,
  relay_delegation_jti text not null,
  billing_subject_id text not null references public.billing_subjects (id),
  ctx_user_id text not null references public.ctx_users (id),
  ctx_org_id text references public.ctx_orgs (id),
  route_id text not null references public.route_configs (id),
  provider_id text not null,
  model_id text not null,
  pricing_version text not null references public.pricing_catalog_versions (version),
  max_estimated_cents bigint not null check (max_estimated_cents > 0),
  max_input_tokens integer check (max_input_tokens is null or max_input_tokens >= 0),
  max_output_tokens integer check (max_output_tokens is null or max_output_tokens >= 0),
  expires_at timestamptz not null,
  created_at timestamptz not null default now()
);

create table if not exists public.usage_reservation_allocations (
  id text primary key,
  reservation_id text not null references public.usage_reservations (id),
  credit_grant_id text not null references public.credit_grants (id),
  credit_source public.ctx_credit_source not null,
  allocated_cents bigint not null check (allocated_cents > 0),
  grant_expires_at timestamptz,
  created_at timestamptz not null default now(),
  unique (reservation_id, credit_grant_id)
);

create table if not exists public.usage_ledger_events (
  id text primary key,
  request_id text not null,
  reservation_id text references public.usage_reservations (id),
  billing_subject_id text not null references public.billing_subjects (id),
  event_kind public.ctx_usage_event_kind not null,
  provider_id text,
  model_id text,
  route_id text references public.route_configs (id),
  provider_request_id text,
  estimated_input_tokens integer check (estimated_input_tokens is null or estimated_input_tokens >= 0),
  estimated_output_tokens integer check (estimated_output_tokens is null or estimated_output_tokens >= 0),
  actual_input_tokens integer check (actual_input_tokens is null or actual_input_tokens >= 0),
  actual_output_tokens integer check (actual_output_tokens is null or actual_output_tokens >= 0),
  provider_cost_micros bigint check (provider_cost_micros is null or provider_cost_micros >= 0),
  billable_cents bigint check (billable_cents is null or billable_cents >= 0),
  error_class text,
  metadata jsonb not null default '{}'::jsonb,
  created_at timestamptz not null default now()
);

create table if not exists public.request_state_events (
  id text primary key,
  request_id text not null,
  from_state public.ctx_relay_request_state,
  to_state public.ctx_relay_request_state not null,
  actor text not null,
  reason text,
  idempotency_key text unique,
  created_at timestamptz not null default now(),
  check (from_state is distinct from to_state)
);

create table if not exists public.audit_events (
  id text primary key,
  billing_subject_id text references public.billing_subjects (id),
  ctx_user_id text references public.ctx_users (id),
  ctx_org_id text references public.ctx_orgs (id),
  event_type text not null,
  subject_type text,
  subject_id text,
  metadata jsonb not null default '{}'::jsonb,
  created_at timestamptz not null default now()
);

create table if not exists public.provider_invoice_lines (
  id text primary key,
  provider_id text not null,
  provider_invoice_id text,
  provider_request_id text,
  matched_request_id text,
  model_id text,
  input_tokens integer check (input_tokens is null or input_tokens >= 0),
  output_tokens integer check (output_tokens is null or output_tokens >= 0),
  provider_cost_micros bigint not null check (provider_cost_micros >= 0),
  invoice_period_start timestamptz,
  invoice_period_end timestamptz,
  metadata jsonb not null default '{}'::jsonb,
  created_at timestamptz not null default now()
);

create index if not exists ctx_accounts_ctx_user_id_idx on public.ctx_accounts (ctx_user_id);
create index if not exists ctx_memberships_ctx_user_id_idx on public.ctx_memberships (ctx_user_id);
create index if not exists ctx_memberships_ctx_org_id_status_idx on public.ctx_memberships (ctx_org_id, status);
create index if not exists billing_entitlements_subject_status_idx on public.billing_entitlements (billing_subject_id, status);
create index if not exists billing_spend_limits_subject_user_period_idx on public.billing_spend_limits (billing_subject_id, ctx_user_id, status, period_start, period_end);
create index if not exists route_configs_subject_status_idx on public.route_configs (billing_subject_id, status);
create index if not exists route_policy_versions_subject_active_idx on public.route_policy_versions (billing_subject_id, is_active);
create index if not exists model_prices_provider_model_idx on public.model_prices (provider_id, model_id);
create index if not exists credit_grants_subject_source_expiry_idx on public.credit_grants (billing_subject_id, source, expires_at);
create index if not exists credit_ledger_events_subject_created_idx on public.credit_ledger_events (billing_subject_id, created_at);
create index if not exists credit_ledger_events_request_idx on public.credit_ledger_events (request_id);
create index if not exists usage_reservations_subject_user_created_idx on public.usage_reservations (billing_subject_id, ctx_user_id, created_at);
create index if not exists usage_reservation_allocations_grant_idx on public.usage_reservation_allocations (credit_grant_id);
create index if not exists usage_ledger_events_request_created_idx on public.usage_ledger_events (request_id, created_at);
create index if not exists usage_ledger_events_subject_created_idx on public.usage_ledger_events (billing_subject_id, created_at);
create index if not exists usage_ledger_events_provider_request_idx on public.usage_ledger_events (provider_request_id);
create index if not exists request_state_events_request_created_idx on public.request_state_events (request_id, created_at);
create index if not exists audit_events_subject_created_idx on public.audit_events (billing_subject_id, created_at);
create index if not exists provider_invoice_lines_provider_request_idx on public.provider_invoice_lines (provider_id, provider_request_id);
create index if not exists provider_invoice_lines_matched_request_idx on public.provider_invoice_lines (matched_request_id);

alter table public.ctx_users enable row level security;
alter table public.ctx_accounts enable row level security;
alter table public.ctx_orgs enable row level security;
alter table public.ctx_memberships enable row level security;
alter table public.billing_subjects enable row level security;
alter table public.billing_entitlements enable row level security;
alter table public.billing_spend_limits enable row level security;
alter table public.route_configs enable row level security;
alter table public.route_policy_versions enable row level security;
alter table public.pricing_catalog_versions enable row level security;
alter table public.model_prices enable row level security;
alter table public.credit_grants enable row level security;
alter table public.credit_ledger_events enable row level security;
alter table public.usage_reservations enable row level security;
alter table public.usage_reservation_allocations enable row level security;
alter table public.usage_ledger_events enable row level security;
alter table public.request_state_events enable row level security;
alter table public.audit_events enable row level security;
alter table public.provider_invoice_lines enable row level security;

revoke all on table
  public.ctx_users,
  public.ctx_accounts,
  public.ctx_orgs,
  public.ctx_memberships,
  public.billing_subjects,
  public.billing_entitlements,
  public.billing_spend_limits,
  public.route_configs,
  public.route_policy_versions,
  public.pricing_catalog_versions,
  public.model_prices,
  public.credit_grants,
  public.credit_ledger_events,
  public.usage_reservations,
  public.usage_reservation_allocations,
  public.usage_ledger_events,
  public.request_state_events,
  public.audit_events,
  public.provider_invoice_lines
from anon, authenticated, public;

grant all on table
  public.ctx_users,
  public.ctx_accounts,
  public.ctx_orgs,
  public.ctx_memberships,
  public.billing_subjects,
  public.billing_entitlements,
  public.billing_spend_limits,
  public.route_configs,
  public.route_policy_versions,
  public.pricing_catalog_versions,
  public.model_prices,
  public.credit_grants,
  public.credit_ledger_events,
  public.usage_reservations,
  public.usage_reservation_allocations,
  public.usage_ledger_events,
  public.request_state_events,
  public.audit_events,
  public.provider_invoice_lines
to service_role;

drop policy if exists ctx_users_service_role on public.ctx_users;
create policy ctx_users_service_role on public.ctx_users for all to service_role using (true) with check (true);

drop policy if exists ctx_accounts_service_role on public.ctx_accounts;
create policy ctx_accounts_service_role on public.ctx_accounts for all to service_role using (true) with check (true);

drop policy if exists ctx_orgs_service_role on public.ctx_orgs;
create policy ctx_orgs_service_role on public.ctx_orgs for all to service_role using (true) with check (true);

drop policy if exists ctx_memberships_service_role on public.ctx_memberships;
create policy ctx_memberships_service_role on public.ctx_memberships for all to service_role using (true) with check (true);

drop policy if exists billing_subjects_service_role on public.billing_subjects;
create policy billing_subjects_service_role on public.billing_subjects for all to service_role using (true) with check (true);

drop policy if exists billing_entitlements_service_role on public.billing_entitlements;
create policy billing_entitlements_service_role on public.billing_entitlements for all to service_role using (true) with check (true);

drop policy if exists billing_spend_limits_service_role on public.billing_spend_limits;
create policy billing_spend_limits_service_role on public.billing_spend_limits for all to service_role using (true) with check (true);

drop policy if exists route_configs_service_role on public.route_configs;
create policy route_configs_service_role on public.route_configs for all to service_role using (true) with check (true);

drop policy if exists route_policy_versions_service_role on public.route_policy_versions;
create policy route_policy_versions_service_role on public.route_policy_versions for all to service_role using (true) with check (true);

drop policy if exists pricing_catalog_versions_service_role on public.pricing_catalog_versions;
create policy pricing_catalog_versions_service_role on public.pricing_catalog_versions for all to service_role using (true) with check (true);

drop policy if exists model_prices_service_role on public.model_prices;
create policy model_prices_service_role on public.model_prices for all to service_role using (true) with check (true);

drop policy if exists credit_grants_service_role on public.credit_grants;
create policy credit_grants_service_role on public.credit_grants for all to service_role using (true) with check (true);

drop policy if exists credit_ledger_events_service_role on public.credit_ledger_events;
create policy credit_ledger_events_service_role on public.credit_ledger_events for all to service_role using (true) with check (true);

drop policy if exists usage_reservations_service_role on public.usage_reservations;
create policy usage_reservations_service_role on public.usage_reservations for all to service_role using (true) with check (true);

drop policy if exists usage_reservation_allocations_service_role on public.usage_reservation_allocations;
create policy usage_reservation_allocations_service_role on public.usage_reservation_allocations for all to service_role using (true) with check (true);

drop policy if exists usage_ledger_events_service_role on public.usage_ledger_events;
create policy usage_ledger_events_service_role on public.usage_ledger_events for all to service_role using (true) with check (true);

drop policy if exists request_state_events_service_role on public.request_state_events;
create policy request_state_events_service_role on public.request_state_events for all to service_role using (true) with check (true);

drop policy if exists audit_events_service_role on public.audit_events;
create policy audit_events_service_role on public.audit_events for all to service_role using (true) with check (true);

drop policy if exists provider_invoice_lines_service_role on public.provider_invoice_lines;
create policy provider_invoice_lines_service_role on public.provider_invoice_lines for all to service_role using (true) with check (true);
