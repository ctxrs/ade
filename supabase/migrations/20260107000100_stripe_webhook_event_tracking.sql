alter table public.billing_subscription
  add column if not exists stripe_last_event_id text,
  add column if not exists stripe_last_event_created timestamptz;
